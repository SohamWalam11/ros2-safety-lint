//! The fix engine: Synthesize → Verify → Diff → Apply.
//!
//! 1. Synthesize: collect `Safe` fixes from diagnostics (suggestions are never applied).
//! 2. Verify: apply them in memory and re-run the full analysis on the patched
//!    workspace. A fix is kept only if its diagnostic disappears and no new
//!    error-level diagnostic, and no new parse failure, appears in the touched files, and
//!    no new finding of any severity points at the text the fix wrote. Fixes that fail are dropped
//!    and the remaining set is verified again.
//! 3. Diff: produce a unified diff for review.
//! 4. Apply (only with `--write`): write each file atomically (temp file + rename).

use crate::config::Config;
use crate::diag::{Diagnostic, FixSafety, TextEdit};
use crate::engine::{self, Analysis, Options};
use crate::rules::Severity;
use crate::source::FileId;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct PlannedFix {
    pub rule: &'static str,
    pub message: String,
    pub description: String,
    pub edits: Vec<TextEdit>,
}

#[derive(Debug, Default)]
pub struct FixReport {
    pub applied: Vec<PlannedFix>,
    pub rejected: Vec<(PlannedFix, String)>,
    /// file path → (original, patched)
    pub files: BTreeMap<PathBuf, (String, String)>,
    pub rel_paths: HashMap<PathBuf, String>,
}

/// Applies edits to text; edits must not overlap (overlapping later ones are skipped).
pub fn apply_edits(text: &str, edits: &[&TextEdit]) -> String {
    let mut sorted: Vec<&&TextEdit> = edits.iter().collect();
    sorted.sort_by_key(|a| (a.start, a.end));
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    for e in sorted {
        if e.start < pos
            || e.end > text.len()
            || !text.is_char_boundary(e.start)
            || !text.is_char_boundary(e.end)
        {
            continue;
        }
        out.push_str(&text[pos..e.start]);
        out.push_str(&e.replacement);
        pos = e.end;
    }
    out.push_str(&text[pos..]);
    out
}

fn overlaps(a: &TextEdit, b: &TextEdit) -> bool {
    a.file == b.file && a.start < b.end && b.start < a.end
}

fn plan(diags: &[Diagnostic], include_assisted: bool) -> Vec<PlannedFix> {
    let mut out: Vec<PlannedFix> = Vec::new();
    for d in diags {
        let Some(f) = &d.fix else { continue };
        let allowed =
            f.safety == FixSafety::Safe || (include_assisted && f.safety == FixSafety::Assisted);
        if !allowed || f.edits.is_empty() {
            continue;
        }
        // Skip duplicates and fixes that overlap an already-planned one.
        if out.iter().any(|p| p.edits == f.edits) {
            continue;
        }
        if out.iter().any(|p| {
            p.edits
                .iter()
                .any(|a| f.edits.iter().any(|b| overlaps(a, b)))
        }) {
            continue;
        }
        out.push(PlannedFix {
            rule: d.rule,
            message: d.message.clone(),
            description: f.description.clone(),
            edits: f.edits.clone(),
        });
    }
    out
}

fn patched_files(a: &Analysis, fixes: &[PlannedFix]) -> HashMap<FileId, String> {
    let mut by_file: HashMap<FileId, Vec<&TextEdit>> = HashMap::new();
    for f in fixes {
        for e in &f.edits {
            by_file.entry(e.file).or_default().push(e);
        }
    }
    by_file
        .into_iter()
        .map(|(fid, edits)| (fid, apply_edits(&a.index.file(fid).text, &edits)))
        .collect()
}

/// Where each fix's replacement text lands in the patched files: (fix index, file, start,
/// end). Edits are ordered exactly as [`apply_edits`] applies them (stable sort by
/// position, in fix order), so the offsets match the patched text.
fn written_spans(fixes: &[PlannedFix]) -> Vec<(usize, FileId, usize, usize)> {
    let mut all: Vec<(usize, &TextEdit)> = fixes
        .iter()
        .enumerate()
        .flat_map(|(i, f)| f.edits.iter().map(move |e| (i, e)))
        .collect();
    all.sort_by_key(|(_, e)| (e.file, e.start, e.end));
    let mut out = Vec::new();
    let mut file = None;
    let mut delta: isize = 0;
    for (i, e) in all {
        if file != Some(e.file) {
            file = Some(e.file);
            delta = 0;
        }
        let start = (e.start as isize + delta) as usize;
        out.push((i, e.file, start, start + e.replacement.len()));
        delta += e.replacement.len() as isize - (e.end - e.start) as isize;
    }
    out
}

/// A finding that a fix must not introduce in a file it touches: any error, and a file
/// that no longer parses (R001 is reported as info, but a fix that causes it corrupts the
/// file).
fn breaks(d: &Diagnostic) -> bool {
    d.severity == Severity::Error || d.rule == "R001"
}

/// Computes verified fixes for the current analysis.
/// `include_assisted`: also verify assistant-chosen fixes (shown in the diff; written
/// only when the caller passes them through `--accept-assist`).
pub fn run(
    root: &Path,
    config: &Config,
    opts: &Options,
    a: &Analysis,
    include_assisted: bool,
) -> FixReport {
    let mut candidates = plan(&a.diagnostics, include_assisted);
    let mut report = FixReport::default();
    // Re-analysis after a patch is a change event: only patched files are parsed again
    // (with `opts.cache`) and only agents that read those kinds of files run again.
    let previous = std::sync::Arc::new(a.snapshot());
    for _round in 0..3 {
        if candidates.is_empty() {
            break;
        }
        let patched = patched_files(a, &candidates);
        let mut o = opts.clone();
        o.previous = Some(previous.clone());
        for (fid, text) in &patched {
            o.overrides
                .insert(a.index.file(*fid).abs_path.clone(), text.clone());
        }
        let after = engine::analyze(root, config, &o);
        let touched: HashSet<String> = patched
            .keys()
            .map(|f| a.index.file(*f).rel_path.clone())
            .collect();
        let old_errors: HashSet<(String, &str, String)> = a
            .diagnostics
            .iter()
            .filter(|d| breaks(d))
            .map(|d| {
                (
                    a.index.file(d.loc.file).rel_path.clone(),
                    d.rule,
                    d.message.clone(),
                )
            })
            .collect();
        let new_errors: Vec<&Diagnostic> = after
            .diagnostics
            .iter()
            .filter(|d| breaks(d) && touched.contains(&after.index.file(d.loc.file).rel_path))
            .filter(|d| {
                !old_errors.contains(&(
                    after.index.file(d.loc.file).rel_path.clone(),
                    d.rule,
                    d.message.clone(),
                ))
            })
            .collect();
        let still: HashSet<(&str, &str)> = after
            .diagnostics
            .iter()
            .map(|d| (d.rule, d.message.as_str()))
            .collect();
        // Findings of any severity that are new after patching; one that lands on the text
        // a fix wrote means the fix itself is wrong (e.g. it adds an unresolvable dependency).
        let old_all: HashSet<(String, &str, &str)> = a
            .diagnostics
            .iter()
            .map(|d| {
                (
                    a.index.file(d.loc.file).rel_path.clone(),
                    d.rule,
                    d.message.as_str(),
                )
            })
            .collect();
        let new_all: Vec<(String, &Diagnostic)> = after
            .diagnostics
            .iter()
            .map(|d| (after.index.file(d.loc.file).rel_path.clone(), d))
            .filter(|(rel, d)| !old_all.contains(&(rel.clone(), d.rule, d.message.as_str())))
            .collect();
        let spans = written_spans(&candidates);
        let mut keep = Vec::new();
        let mut dropped = false;
        for (i, f) in candidates.into_iter().enumerate() {
            let file_rel = a.index.file(f.edits[0].file).rel_path.clone();
            let on_written_text = new_all.iter().find(|(rel, d)| {
                spans.iter().any(|(j, fid, s, e)| {
                    *j == i
                        && *rel == a.index.file(*fid).rel_path
                        && d.loc.start <= *e
                        && d.loc.end >= *s
                })
            });
            if still.contains(&(f.rule, f.message.as_str())) {
                report
                    .rejected
                    .push((f, "the finding is still reported after the fix".into()));
                dropped = true;
            } else if let Some(d) = new_errors
                .iter()
                .find(|d| after.index.file(d.loc.file).rel_path == file_rel)
            {
                let why = format!("introduces a new error: {}", d.message);
                report.rejected.push((f, why));
                dropped = true;
            } else if let Some((_, d)) = on_written_text {
                let why = format!(
                    "the text it writes is reported itself ({}): {}",
                    d.rule, d.message
                );
                report.rejected.push((f, why));
                dropped = true;
            } else {
                keep.push(f);
            }
        }
        if !dropped {
            report.applied = keep;
            break;
        }
        candidates = keep;
    }
    for (fid, text) in patched_files(a, &report.applied) {
        let f = a.index.file(fid);
        report
            .rel_paths
            .insert(f.abs_path.clone(), f.rel_path.clone());
        report
            .files
            .insert(f.abs_path.clone(), (f.text.clone(), text));
    }
    report
}

/// Unified diff of all patched files.
pub fn unified_diff(r: &FixReport) -> String {
    let mut out = String::new();
    for (path, (old, new)) in &r.files {
        let rel = r
            .rel_paths
            .get(path)
            .cloned()
            .unwrap_or_else(|| path.display().to_string());
        let diff = similar::TextDiff::from_lines(old, new);
        out.push_str(
            &diff
                .unified_diff()
                .context_radius(3)
                .header(&format!("a/{rel}"), &format!("b/{rel}"))
                .to_string(),
        );
    }
    out
}

/// Writes patched files atomically (write to a temp file in the same dir, then rename).
pub fn write(r: &FixReport) -> std::io::Result<usize> {
    for (path, (_, new)) in &r.files {
        let dir = path.parent().unwrap_or(Path::new("."));
        let tmp = dir.join(format!(
            ".{}.rosfix.tmp",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("file")
        ));
        std::fs::write(&tmp, new)?;
        if let Err(e) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    }
    Ok(r.files.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_spans_match_the_patched_text() {
        let text = "<p><n>a</n></p>";
        let fix = |at: usize, s: &str| PlannedFix {
            rule: "L002",
            message: String::new(),
            description: String::new(),
            edits: vec![TextEdit {
                file: 0,
                start: at,
                end: at,
                replacement: s.into(),
            }],
        };
        let fixes = vec![fix(11, "<d>x</d>"), fix(11, "<d>y</d>"), fix(1, "Q")];
        let patched = apply_edits(
            text,
            &fixes
                .iter()
                .flat_map(|f| f.edits.iter())
                .collect::<Vec<_>>(),
        );
        for (i, _, s, e) in written_spans(&fixes) {
            assert_eq!(&patched[s..e], fixes[i].edits[0].replacement, "{patched}");
        }
    }

    #[test]
    fn edits_apply_in_order_and_skip_overlaps() {
        let e1 = TextEdit {
            file: 0,
            start: 0,
            end: 1,
            replacement: "X".into(),
        };
        let e2 = TextEdit {
            file: 0,
            start: 3,
            end: 3,
            replacement: "+".into(),
        };
        let e3 = TextEdit {
            file: 0,
            start: 0,
            end: 2,
            replacement: "no".into(),
        };
        assert_eq!(apply_edits("abcd", &[&e2, &e1]), "Xbc+d");
        assert_eq!(apply_edits("abcd", &[&e1, &e3]), "Xbcd");
    }
}
