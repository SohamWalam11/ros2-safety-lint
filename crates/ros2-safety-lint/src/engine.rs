//! The analysis pipeline: index → resolve → agents → post-processing.

use crate::agents::{self, AgentRun, Context};
use crate::config::{Config, RuleSetting};
use crate::diag::Diagnostic;
use crate::resolve::{self, Graph};
use crate::source::SourceFile;
use crate::util::fnv1a;
use crate::workspace::{self, Index, IndexStats};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub jobs: usize,
    pub agents: Option<Vec<String>>,
    /// In-memory file contents that replace what is on disk (used to verify fixes).
    pub overrides: HashMap<PathBuf, String>,
    /// Installed ROS prefixes to read (normally `AMENT_PREFIX_PATH`).
    pub prefixes: Vec<PathBuf>,
    /// ROS distribution override (otherwise `ROS_DISTRO` / inferred from the prefix).
    pub distro: Option<String>,
    /// Per-file facts from earlier analyses: unchanged files are not parsed again.
    pub cache: Option<std::sync::Arc<workspace::ExtractCache>>,
    /// The previous analysis of the same workspace with the same configuration. Agents
    /// whose inputs did not change since then reuse their findings instead of running.
    pub previous: Option<std::sync::Arc<Snapshot>>,
}

/// What a re-analysis needs from the previous one: file fingerprints and raw findings.
#[derive(Debug, Default)]
pub struct Snapshot {
    files: Vec<(String, workspace::FileKind, u64)>,
    /// Launch graphs of the previous run (reused when their inputs did not change).
    roots: Vec<resolve::RootGraph>,
    by_agent: HashMap<&'static str, Vec<Diagnostic>>,
    agents: Option<Vec<String>>,
}

fn fingerprints(ix: &Index) -> Vec<(String, workspace::FileKind, u64)> {
    (0..ix.sources.files.len())
        .map(|i| (ix.file(i).rel_path.clone(), ix.kinds[i], ix.content_hash(i)))
        .collect()
}

/// The change event between a previous analysis and a new index: the kinds of files whose
/// content changed. None when the set of files differs (ids would not line up): then
/// everything is recomputed.
fn changed_kinds(
    prev: &Snapshot,
    now: &[(String, workspace::FileKind, u64)],
) -> Option<std::collections::BTreeSet<workspace::FileKind>> {
    if prev.files.len() != now.len() {
        return None;
    }
    let mut changed = std::collections::BTreeSet::new();
    for (a, b) in prev.files.iter().zip(now) {
        if a.0 != b.0 || a.1 != b.1 {
            return None;
        }
        if a.2 != b.2 {
            changed.insert(b.1);
        }
    }
    Some(changed)
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Timings {
    pub index_ms: u128,
    pub resolve_ms: u128,
    pub agents_ms: u128,
}

pub struct Analysis {
    pub index: Index,
    pub graph: Graph,
    pub diagnostics: Vec<Diagnostic>,
    pub runs: Vec<AgentRun>,
    pub timings: Timings,
    pub suppressed: usize,
    /// Raw findings per agent and file fingerprints, for the next change event.
    by_agent: HashMap<&'static str, Vec<Diagnostic>>,
    agents: Option<Vec<String>>,
}

impl Analysis {
    pub fn stats(&self) -> &IndexStats {
        &self.index.stats
    }

    /// State to pass as [`Options::previous`] when re-analysing after an edit.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            files: fingerprints(&self.index),
            roots: self.graph.roots.clone(),
            by_agent: self.by_agent.clone(),
            agents: self.agents.clone(),
        }
    }
}

pub fn analyze(root: &Path, config: &Config, opts: &Options) -> Analysis {
    let t0 = Instant::now();
    let index = workspace::index(
        root,
        config,
        &workspace::IndexOptions {
            jobs: opts.jobs,
            overrides: opts.overrides.clone(),
            prefixes: opts.prefixes.clone(),
            distro: opts.distro.clone(),
            cache: opts.cache.clone(),
        },
    );
    let t1 = Instant::now();
    let now_files = fingerprints(&index);
    // Launch graphs can be reused when only launch/parameter/robot files changed: node
    // implementations (code, manifests, build files) must be the same as last time.
    let reusable = opts.previous.as_deref().filter(|p| p.files.len() == now_files.len() && p.files.iter().zip(&now_files).all(|(a, b)| a.0 == b.0 && a.1 == b.1));
    let changed_ids: std::collections::HashSet<usize> = reusable
        .map(|p| p.files.iter().zip(&now_files).enumerate().filter(|(_, (a, b))| a.2 != b.2).map(|(i, _)| i).collect())
        .unwrap_or_default();
    let code_changed = changed_ids.iter().any(|i| {
        use workspace::FileKind as K;
        matches!(now_files[*i].1, K::Cpp | K::Python | K::Manifest | K::CMakeLists | K::SetupPy | K::SetupCfg | K::Other)
    });
    let graph = match reusable {
        Some(p) if !code_changed => resolve::build_reusing(&index, Some((&p.roots, &changed_ids))),
        _ => resolve::build(&index),
    };
    let t2 = Instant::now();
    let cx = Context {
        ix: &index,
        graph: &graph,
        config,
    };
    // The change event: only meaningful against a previous run with the same agent selection.
    let changed = opts
        .previous
        .as_deref()
        .filter(|p| p.agents == opts.agents)
        .and_then(|p| changed_kinds(p, &now_files).map(|c| (c, p)));
    let event = changed.as_ref().map(|(c, p)| agents::ChangeEvent {
        changed: c,
        previous: &p.by_agent,
    });
    let (raw, runs, by_agent) = agents::run(&cx, opts.agents.as_deref(), event.as_ref());
    let t3 = Instant::now();
    let (mut diagnostics, suppressed) = postprocess(&index, config, raw);
    add_include_context(&index, &graph, &mut diagnostics);
    Analysis {
        index,
        graph,
        diagnostics,
        runs,
        timings: Timings {
            index_ms: (t1 - t0).as_millis(),
            resolve_ms: (t2 - t1).as_millis(),
            agents_ms: (t3 - t2).as_millis(),
        },
        suppressed,
        by_agent,
        agents: opts.agents.clone(),
    }
}

/// A finding inside a launch file that other launch files include gets the include
/// statements as related locations, so the report shows how the file is reached (a launch
/// error in an included file surfaces when the including file is launched).
fn add_include_context(ix: &Index, graph: &Graph, diags: &mut [Diagnostic]) {
    const SHOWN: usize = 3;
    let mut by_file: HashMap<crate::source::FileId, Vec<crate::source::Loc>> = HashMap::new();
    for r in &graph.roots {
        for (f, loc) in &r.includes {
            let v = by_file.entry(*f).or_default();
            if !v.contains(loc) {
                v.push(*loc);
            }
        }
    }
    for d in diags.iter_mut() {
        let Some(locs) = by_file.get(&d.loc.file) else { continue };
        let name = ix.file(d.loc.file).rel_path.rsplit('/').next().unwrap_or("").to_string();
        for (i, l) in locs.iter().enumerate().take(SHOWN) {
            if d.related.iter().any(|r| r.loc == *l) {
                continue;
            }
            let more = if i + 1 == SHOWN && locs.len() > SHOWN {
                format!(" (and {} more)", locs.len() - SHOWN)
            } else {
                String::new()
            };
            d.related.push(crate::diag::Related { loc: *l, message: format!("`{name}` is included here{more}") });
        }
    }
}

/// `rosfix: ignore` or `rosfix: ignore[P003, Q001]` on the same line, or on the previous
/// line when that line is only the comment. A trailing comment on the previous line
/// belongs to that line's code and does not hide findings on the next one.
fn suppressed(file: &SourceFile, line: usize, rule: &str) -> bool {
    for l in [line, line.saturating_sub(1)] {
        if l == 0 {
            continue;
        }
        let text = file.line_text(l);
        if l != line {
            let t = text.trim_start();
            if !(t.starts_with('#') || t.starts_with("//") || t.starts_with("<!--") || t.starts_with("/*")) {
                continue;
            }
        }
        if let Some(i) = text.find("rosfix: ignore") {
            let rest = &text[i + "rosfix: ignore".len()..];
            match rest.strip_prefix('[') {
                Some(r) => {
                    let list = r.split(']').next().unwrap_or("");
                    if list.split(',').any(|x| x.trim().eq_ignore_ascii_case(rule)) {
                        return true;
                    }
                }
                None => return true,
            }
        }
    }
    false
}

fn postprocess(ix: &Index, config: &Config, raw: Vec<Diagnostic>) -> (Vec<Diagnostic>, usize) {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut n_suppressed = 0;
    for mut d in raw {
        match config.rule_setting(d.rule) {
            RuleSetting::Off => continue,
            RuleSetting::On(sev) => {
                // A rule's severity override replaces the default, but agents may have
                // deliberately lowered a specific finding; keep the lower of the two
                // only when the config did not explicitly set one.
                if config.rule_overrides.contains_key(d.rule) {
                    d.severity = sev;
                }
            }
        }
        // Installed packages are context, not the user's code.
        if ix.external_files.contains(&d.loc.file) {
            continue;
        }
        let file = ix.file(d.loc.file);
        let (line, _) = file.line_col(d.loc.start);
        if suppressed(file, line, d.rule) {
            n_suppressed += 1;
            continue;
        }
        if !seen.insert((d.rule, d.loc, d.message.clone())) {
            continue;
        }
        out.push(d);
    }
    // The Nav2 typo rule (catalog-based) is redundant where the source-based rule already fired.
    let p003: HashSet<crate::source::Loc> = out
        .iter()
        .filter(|d| d.rule == "P003")
        .map(|d| d.loc)
        .collect();
    out.retain(|d| !(d.rule == "N006" && p003.contains(&d.loc)));
    out.sort_by(|a, b| {
        let fa = &ix.file(a.loc.file).rel_path;
        let fb = &ix.file(b.loc.file).rel_path;
        fa.cmp(fb)
            .then(a.loc.start.cmp(&b.loc.start))
            .then(a.rule.cmp(b.rule))
    });
    (out, n_suppressed)
}

/// Stable fingerprint for baselines: rule + file + message (no line numbers).
pub fn fingerprint(ix: &Index, d: &Diagnostic) -> String {
    let key = format!("{}|{}|{}", d.rule, ix.file(d.loc.file).rel_path, d.message);
    format!("{:016x}", fnv1a(key.as_bytes()))
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct Baseline {
    pub version: u32,
    pub fingerprints: Vec<String>,
}

pub fn write_baseline(path: &Path, a: &Analysis) -> std::io::Result<usize> {
    let mut fps: Vec<String> = a
        .diagnostics
        .iter()
        .map(|d| fingerprint(&a.index, d))
        .collect();
    fps.sort();
    fps.dedup();
    let n = fps.len();
    let b = Baseline {
        version: 1,
        fingerprints: fps,
    };
    std::fs::write(path, serde_json::to_string_pretty(&b).unwrap_or_default())?;
    Ok(n)
}

/// Removes diagnostics present in the baseline; returns how many were hidden.
pub fn apply_baseline(path: &Path, a: &mut Analysis) -> Result<usize, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let b: Baseline =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let set: HashSet<String> = b.fingerprints.into_iter().collect();
    let before = a.diagnostics.len();
    let ix = &a.index;
    a.diagnostics.retain(|d| !set.contains(&fingerprint(ix, d)));
    Ok(before - a.diagnostics.len())
}

#[cfg(test)]
mod tests {
    use crate::testutil::analyze_files;

    const CPP: &str = "#include <rclcpp/rclcpp.hpp>\nclass N : public rclcpp::Node { public: N() : Node(\"n\") {\n declare_parameter(\"a\", 1.0);\n declare_parameter(\"b\", 1.0);\n declare_parameter(\"c\", 1.0);\n} };\n";

    fn p004_lines(yaml: &str) -> Vec<String> {
        let d = analyze_files(&[
            ("q/package.xml", "<package format=\"3\"><name>q</name></package>"),
            ("q/CMakeLists.txt", "project(q)\nadd_executable(n src/n.cpp)\ninstall(TARGETS n DESTINATION lib/${PROJECT_NAME})\n"),
            ("q/src/n.cpp", CPP),
            ("q/config/p.yaml", yaml),
            ("q/launch/l.launch.xml", "<launch><node pkg=\"q\" exec=\"n\"><param from=\"$(find-pkg-share q)/config/p.yaml\"/></node></launch>"),
        ]);
        let mut v: Vec<String> = d.iter().filter(|x| x.rule == "P004").map(|x| x.message.split('`').nth(1).unwrap_or("").to_string()).collect();
        v.sort();
        v
    }

    #[test]
    fn findings_in_included_files_name_the_include() {
        let child = "<launch>\n<node pkg=\"q\" exec=\"nope\"/>\n</launch>\n";
        let parent = "<launch>\n<include file=\"$(find-pkg-share q)/launch/child.launch.xml\"/>\n</launch>\n";
        let d = analyze_files(&[
            ("q/package.xml", "<package format=\"3\"><name>q</name></package>"),
            ("q/CMakeLists.txt", "project(q)\nadd_executable(n src/n.cpp)\ninstall(TARGETS n DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch DESTINATION share/${PROJECT_NAME})\n"),
            ("q/src/n.cpp", CPP),
            ("q/launch/child.launch.xml", child),
            ("q/launch/parent.launch.xml", parent),
        ]);
        let l001 = d.iter().find(|x| x.rule == "L001").unwrap_or_else(|| panic!("{d:#?}"));
        let inc: Vec<&str> = l001.related.iter().map(|r| r.message.as_str()).filter(|m| m.contains("is included here")).collect();
        assert_eq!(inc, ["`child.launch.xml` is included here"], "{l001:#?}");
    }

    #[test]
    fn trailing_suppression_does_not_hide_the_next_line() {
        let y = "n:\n  ros__parameters:\n    a: 1  # rosfix: ignore[P004]\n    b: 1\n    c: 1\n";
        assert_eq!(p004_lines(y), ["b", "c"]);
    }

    #[test]
    fn comment_line_suppresses_the_next_line() {
        let y = "n:\n  ros__parameters:\n    # rosfix: ignore[P004]\n    a: 1\n    b: 1\n    c: 1  # rosfix: ignore\n";
        assert_eq!(p004_lines(y), ["b"]);
    }
}
