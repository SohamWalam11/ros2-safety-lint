//! Long-running analysis on top of the incremental engine: `--watch` and `--lsp`.
//!
//! A [`Session`] keeps the parse cache and the previous run's snapshot, so every
//! re-analysis is a change event: only edited files are parsed again and only agents that
//! read those kinds of files run again (see `agents::run`).

use crate::config::Config;
use crate::diag::Diagnostic;
use crate::engine::{analyze, fingerprint, Analysis, Options};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

pub struct Session {
    root: PathBuf,
    config: Config,
    opts: Options,
    pub last: Analysis,
    /// Re-analysis time of the last update, in milliseconds.
    pub last_ms: u128,
}

/// Findings that appeared or disappeared with one update: (fingerprint, display line).
#[derive(Debug, Default)]
pub struct Delta {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

/// A path in one spelling for comparisons (`/` separators, no `\\?\`, lower case on Windows).
fn norm_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s.strip_prefix("//?/").unwrap_or(&s).to_string();
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s
    }
}

fn line(a: &Analysis, d: &Diagnostic) -> String {
    let f = a.index.file(d.loc.file);
    let (l, c) = f.line_col(d.loc.start);
    format!(
        "{}[{}] {}:{l}:{c}: {}",
        d.severity.as_str(),
        d.rule,
        f.rel_path,
        d.message
    )
}

fn keyed(a: &Analysis) -> BTreeMap<String, String> {
    a.diagnostics
        .iter()
        .map(|d| (fingerprint(&a.index, d), line(a, d)))
        .collect()
}

impl Session {
    pub fn new(root: &Path, config: Config, mut opts: Options) -> Self {
        opts.cache = Some(Default::default());
        let t = Instant::now();
        let last = analyze(root, &config, &opts);
        Session {
            root: root.to_path_buf(),
            config,
            opts,
            last,
            last_ms: t.elapsed().as_millis(),
        }
    }

    /// Re-analyses with in-memory file contents (unsaved editor buffers) and returns what changed.
    pub fn reanalyze(&mut self, overrides: HashMap<PathBuf, String>) -> Delta {
        // Editors and callers spell paths differently (`/` vs `\`, drive-letter case): use the
        // indexer's own path for each overridden file.
        let known: HashMap<String, PathBuf> = self
            .last
            .index
            .sources
            .files
            .iter()
            .map(|f| (norm_path(&f.abs_path), f.abs_path.clone()))
            .collect();
        let mut o = self.opts.clone();
        o.overrides = overrides
            .into_iter()
            .map(|(p, t)| (known.get(&norm_path(&p)).cloned().unwrap_or(p), t))
            .collect();
        o.previous = Some(Arc::new(self.last.snapshot()));
        let t = Instant::now();
        let next = analyze(&self.root, &self.config, &o);
        self.last_ms = t.elapsed().as_millis();
        let (before, after) = (keyed(&self.last), keyed(&next));
        let delta = Delta {
            added: after
                .iter()
                .filter(|(k, _)| !before.contains_key(*k))
                .map(|(_, v)| v.clone())
                .collect(),
            removed: before
                .iter()
                .filter(|(k, _)| !after.contains_key(*k))
                .map(|(_, v)| v.clone())
                .collect(),
        };
        self.last = next;
        delta
    }
}

// ---------------------------------------------------------------------------- --watch

/// Modification times of every file the analysis looked at, plus the directory listing
/// (new or deleted files change the set).
fn stamps(root: &Path) -> BTreeMap<PathBuf, (SystemTime, u64)> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            !matches!(
                e.file_name().to_str(),
                Some(".git" | "build" | "install" | "log" | "target")
            )
        })
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            e.metadata().ok().map(|m| {
                (
                    e.path().to_path_buf(),
                    (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()),
                )
            })
        })
        .collect()
}

/// `rosfix --watch`: prints the findings once, then the findings that appear (`+`) or
/// disappear (`-`) whenever a file changes. Runs until interrupted.
pub fn watch(root: &Path, config: Config, opts: Options, interval_ms: u64) -> ! {
    let mut s = Session::new(root, config, opts);
    for d in &s.last.diagnostics {
        println!("  {}", line(&s.last, d));
    }
    eprintln!(
        "rosfix: watching {} — {} finding(s), analysed in {} ms",
        root.display(),
        s.last.diagnostics.len(),
        s.last_ms
    );
    let mut seen = stamps(root);
    loop {
        std::thread::sleep(std::time::Duration::from_millis(interval_ms));
        let now = stamps(root);
        if now == seen {
            continue;
        }
        seen = now;
        let d = s.reanalyze(HashMap::new());
        for l in &d.removed {
            println!("- {l}");
        }
        for l in &d.added {
            println!("+ {l}");
        }
        let reused = s.last.runs.iter().filter(|r| r.reused).count();
        eprintln!(
            "rosfix: {} new, {} fixed, {} total — re-analysed in {} ms ({} file(s) parsed, {} agent(s) reused)",
            d.added.len(),
            d.removed.len(),
            s.last.diagnostics.len(),
            s.last_ms,
            s.last.stats().files_parsed,
            reused
        );
    }
}

// ---------------------------------------------------------------------------- --lsp

fn read_message(input: &mut impl std::io::BufRead) -> Option<serde_json::Value> {
    let mut len = None;
    loop {
        let mut h = String::new();
        if input.read_line(&mut h).ok()? == 0 {
            return None;
        }
        let h = h.trim();
        if h.is_empty() {
            break;
        }
        if let Some(v) = h.strip_prefix("Content-Length:") {
            len = v.trim().parse::<usize>().ok();
        }
    }
    let mut buf = vec![0u8; len?];
    std::io::Read::read_exact(input, &mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

fn write_message(out: &mut impl std::io::Write, v: &serde_json::Value) {
    let body = v.to_string();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = out.flush();
}

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let p = uri.strip_prefix("file://")?;
    let p = percent_decode(p);
    // file:///d:/x on Windows
    let p = if p.len() > 2 && p.as_bytes()[0] == b'/' && p.as_bytes()[2] == b':' {
        p[1..].to_string()
    } else {
        p
    };
    Some(PathBuf::from(p))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn path_to_uri(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s.strip_prefix("//?/").unwrap_or(&s).to_string();
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

/// LSP diagnostics for every file with findings, plus empty lists for files that had some
/// before (so editors clear them).
/// The LSP range (0-based lines and characters) of a byte span in a loaded file.
fn lsp_range(
    a: &Analysis,
    file: crate::source::FileId,
    start: usize,
    end: usize,
) -> serde_json::Value {
    let f = a.index.file(file);
    let (l0, c0) = f.line_col(start);
    let (l1, c1) = f.line_col(end.max(start));
    serde_json::json!({"start": {"line": l0.saturating_sub(1), "character": c0.saturating_sub(1)}, "end": {"line": l1.saturating_sub(1), "character": c1.saturating_sub(1)}})
}

/// A finding as an LSP diagnostic.
fn lsp_diagnostic(a: &Analysis, d: &Diagnostic) -> serde_json::Value {
    let severity = match d.severity {
        crate::rules::Severity::Error => 1,
        crate::rules::Severity::Warning => 2,
        crate::rules::Severity::Info => 3,
    };
    let mut msg = d.message.clone();
    if let Some(h) = &d.help {
        msg.push('\n');
        msg.push_str(h);
    }
    serde_json::json!({
        "range": lsp_range(a, d.loc.file, d.loc.start, d.loc.end),
        "severity": severity,
        "code": d.rule,
        "source": "rosfix",
        "message": msg,
    })
}

/// Quick fixes for the findings in `uri` whose lines overlap `range`: one code action per
/// fix, its edits as a workspace edit. Safe fixes are marked preferred.
fn code_actions(a: &Analysis, uri: &str, range: &serde_json::Value) -> Vec<serde_json::Value> {
    let Some(path) = uri_to_path(uri) else {
        return Vec::new();
    };
    let want = norm_path(&path);
    let line = |p: &serde_json::Value| p["line"].as_u64().unwrap_or(0);
    let (from, to) = (line(&range["start"]), line(&range["end"]));
    let mut actions = Vec::new();
    for d in &a.diagnostics {
        let Some(fix) = &d.fix else { continue };
        let f = a.index.file(d.loc.file);
        if f.abs_path.as_os_str().is_empty() || norm_path(&f.abs_path) != want {
            continue;
        }
        let diag = lsp_diagnostic(a, d);
        if line(&diag["range"]["end"]) < from || line(&diag["range"]["start"]) > to {
            continue;
        }
        let mut changes: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
        for e in &fix.edits {
            let ef = a.index.file(e.file);
            changes
                .entry(path_to_uri(&ef.abs_path))
                .or_default()
                .push(serde_json::json!({"range": lsp_range(a, e.file, e.start, e.end), "newText": e.replacement}));
        }
        actions.push(serde_json::json!({
            "title": format!("{} ({})", fix.description, d.rule),
            "kind": "quickfix",
            "diagnostics": [diag],
            "isPreferred": fix.safety == crate::diag::FixSafety::Safe,
            "edit": {"changes": changes},
        }));
    }
    actions
}

fn publish(s: &Session, previously: &mut Vec<String>, out: &mut impl std::io::Write) {
    let a = &s.last;
    let mut by_file: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for d in &a.diagnostics {
        let f = a.index.file(d.loc.file);
        if f.abs_path.as_os_str().is_empty() {
            continue; // knowledge-pack pseudo files
        }
        by_file
            .entry(path_to_uri(&f.abs_path))
            .or_default()
            .push(lsp_diagnostic(a, d));
    }
    for uri in previously.iter() {
        by_file.entry(uri.clone()).or_default();
    }
    for (uri, diags) in &by_file {
        write_message(
            out,
            &serde_json::json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": uri, "diagnostics": diags}}),
        );
    }
    *previously = by_file
        .into_iter()
        .filter(|(_, d)| !d.is_empty())
        .map(|(u, _)| u)
        .collect();
}

/// `rosfix --lsp`: a minimal language server over stdio. Diagnostics are published for the
/// whole workspace after `initialized`, and again after every change to an open document
/// (unsaved contents are analysed as in-memory overrides) or save.
pub fn lsp(
    root: &Path,
    config: Config,
    opts: Options,
    input: &mut impl std::io::BufRead,
    out: &mut impl std::io::Write,
) {
    let mut session: Option<Session> = None;
    let mut open: HashMap<PathBuf, String> = HashMap::new();
    let mut published = Vec::new();
    let (mut config, mut opts) = (Some(config), Some(opts));
    while let Some(msg) = read_message(input) {
        let method = msg["method"].as_str().unwrap_or_default();
        let id = msg.get("id").cloned();
        match method {
            "initialize" => write_message(
                out,
                &serde_json::json!({"jsonrpc": "2.0", "id": id, "result": {
                    "capabilities": {
                        "textDocumentSync": {"openClose": true, "change": 1, "save": {"includeText": false}},
                        "codeActionProvider": {"codeActionKinds": ["quickfix"]}
                    },
                    "serverInfo": {"name": "rosfix", "version": env!("CARGO_PKG_VERSION")}
                }}),
            ),
            "initialized" => {
                if let (Some(c), Some(o)) = (config.take(), opts.take()) {
                    let s = Session::new(root, c, o);
                    publish(&s, &mut published, out);
                    session = Some(s);
                }
            }
            "textDocument/didOpen"
            | "textDocument/didChange"
            | "textDocument/didSave"
            | "textDocument/didClose" => {
                let p = &msg["params"];
                let Some(path) = p["textDocument"]["uri"].as_str().and_then(uri_to_path) else {
                    continue;
                };
                match method {
                    "textDocument/didOpen" => {
                        if let Some(t) = p["textDocument"]["text"].as_str() {
                            open.insert(path, t.to_string());
                        }
                    }
                    "textDocument/didChange" => {
                        if let Some(t) = p["contentChanges"]
                            .as_array()
                            .and_then(|c| c.last())
                            .and_then(|c| c["text"].as_str())
                        {
                            open.insert(path, t.to_string());
                        }
                    }
                    "textDocument/didClose" => {
                        open.remove(&path);
                    }
                    _ => {}
                }
                if let Some(s) = session.as_mut() {
                    s.reanalyze(open.clone());
                    publish(s, &mut published, out);
                }
            }
            "textDocument/codeAction" => {
                let p = &msg["params"];
                let actions = match (&session, p["textDocument"]["uri"].as_str()) {
                    (Some(s), Some(uri)) => code_actions(&s.last, uri, &p["range"]),
                    _ => Vec::new(),
                };
                write_message(
                    out,
                    &serde_json::json!({"jsonrpc": "2.0", "id": id, "result": actions}),
                );
            }
            "shutdown" => write_message(
                out,
                &serde_json::json!({"jsonrpc": "2.0", "id": id, "result": null}),
            ),
            "exit" => return,
            _ => {
                // Unknown requests get an empty result; notifications are ignored.
                if id.is_some() && !method.is_empty() {
                    write_message(
                        out,
                        &serde_json::json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}}),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A parameter file whose second section misspells `ros__parameters` (P002). The first
    /// section keeps it recognisable as a parameter file.
    const BAD: &str = "m:\n  ros__parameters:\n    y: 1\nn:\n  ros__parametres:\n    x: 1\n";

    fn ws() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let w = |r: &str, t: &str| {
            let p = d.path().join(r);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        w(
            "p/package.xml",
            "<package format=\"3\"><name>p</name></package>",
        );
        w(
            "p/config/a.yaml",
            BAD.replace("parametres", "parameters").as_str(),
        );
        d
    }

    #[test]
    fn session_reports_new_and_fixed_findings() {
        let d = ws();
        let mut s = Session::new(d.path(), Config::default(), Options::default());
        assert!(s.last.diagnostics.iter().all(|x| x.rule != "P002"));
        let yaml = d.path().join("p/config/a.yaml");
        let bad = HashMap::from([(yaml.clone(), BAD.to_string())]);
        let delta = s.reanalyze(bad);
        assert!(
            delta.added.iter().any(|l| l.contains("[P002]")),
            "{delta:?}"
        );
        assert!(
            s.last.stats().files_reused > 0,
            "unchanged files must come from the cache"
        );
        let delta = s.reanalyze(HashMap::new());
        assert!(
            delta.removed.iter().any(|l| l.contains("[P002]")),
            "{delta:?}"
        );
    }

    #[test]
    fn lsp_publishes_diagnostics_for_unsaved_changes() {
        let d = ws();
        let yaml = d.path().join("p/config/a.yaml");
        let uri = path_to_uri(&yaml);
        let msgs = [
            serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            serde_json::json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
            serde_json::json!({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {"textDocument": {"uri": uri, "text": BAD}}}),
            serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown"}),
            serde_json::json!({"jsonrpc": "2.0", "method": "exit"}),
        ];
        let mut input = Vec::new();
        for m in &msgs {
            let b = m.to_string();
            input.extend(format!("Content-Length: {}\r\n\r\n{b}", b.len()).into_bytes());
        }
        let mut out = Vec::new();
        lsp(
            d.path(),
            Config::default(),
            Options::default(),
            &mut std::io::Cursor::new(input),
            &mut out,
        );
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\"capabilities\""), "{text}");
        assert!(
            text.contains("publishDiagnostics") && text.contains("P002"),
            "{text}"
        );
        assert!(
            text.contains(&uri) || text.to_lowercase().contains(&uri.to_lowercase()),
            "{text}"
        );
    }

    #[test]
    fn lsp_offers_fixes_as_code_actions() {
        let d = ws();
        let uri = path_to_uri(&d.path().join("p/config/a.yaml"));
        let range = serde_json::json!({"start": {"line": 0, "character": 0}, "end": {"line": 9, "character": 0}});
        let msgs = [
            serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            serde_json::json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
            serde_json::json!({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {"textDocument": {"uri": uri, "text": BAD}}}),
            serde_json::json!({"jsonrpc": "2.0", "id": 7, "method": "textDocument/codeAction", "params": {"textDocument": {"uri": uri}, "range": range, "context": {"diagnostics": []}}}),
            serde_json::json!({"jsonrpc": "2.0", "method": "exit"}),
        ];
        let mut input = Vec::new();
        for m in &msgs {
            let b = m.to_string();
            input.extend(format!("Content-Length: {}\r\n\r\n{b}", b.len()).into_bytes());
        }
        let mut out = Vec::new();
        lsp(
            d.path(),
            Config::default(),
            Options::default(),
            &mut std::io::Cursor::new(input),
            &mut out,
        );
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("codeActionProvider"), "{text}");
        // The response to request 7 carries the P002 rename as a preferred quick fix.
        let reply = text
            .split("Content-Length")
            .find(|m| m.contains("\"id\":7"))
            .expect("code action reply");
        let body: serde_json::Value =
            serde_json::from_str(&reply[reply.find('{').unwrap()..]).unwrap();
        let action = body["result"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["title"].as_str().unwrap().contains("P002"))
            .unwrap_or_else(|| panic!("{body}"));
        assert_eq!(action["kind"], "quickfix");
        assert_eq!(action["isPreferred"], true);
        let edits = action["edit"]["changes"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap();
        assert_eq!(edits[0]["newText"], "ros__parameters");
    }
}
