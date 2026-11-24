//! `rosfix report`: one self-contained HTML page with the findings, the agent status and
//! the launch graph, for CI artifacts, reviews and demos.
//!
//! The page has no external dependencies except mermaid.js for drawing the launch graph;
//! without it (offline) the graph is shown as its Mermaid source.

use crate::engine::Analysis;
use crate::rules::{self, Severity};
use crate::status::{State, Status};
use std::collections::BTreeMap;
use std::fmt::Write;

const MERMAID: &str = "https://cdn.jsdelivr.net/npm/mermaid@10.9.1/dist/mermaid.min.js";
/// Above this many node instances the graph is too large to read as one drawing.
const GRAPH_LIMIT: usize = 150;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const STYLE: &str = r#"
:root { --bg: #ffffff; --fg: #1d2127; --muted: #5b6470; --line: #d9dde3; --panel: #f5f6f8;
        --err: #b42318; --warn: #a15c07; --info: #2f5f9e; --ok: #22743d; }
@media (prefers-color-scheme: dark) {
  :root { --bg: #15181c; --fg: #e4e7eb; --muted: #9aa3ad; --line: #2d333b; --panel: #1c2026;
          --err: #f97066; --warn: #fdb022; --info: #84adff; --ok: #6ce9a6; }
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--fg);
       font: 15px/1.5 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif; }
main { max-width: 1100px; margin: 0 auto; padding: 32px 16px 64px; }
h1 { font-size: 24px; margin: 0 0 4px; }
h2 { font-size: 18px; margin: 40px 0 12px; padding-bottom: 6px; border-bottom: 1px solid var(--line); }
h3 { font-size: 15px; margin: 24px 0 8px; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
p.meta { color: var(--muted); margin: 0; }
.cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 12px; margin-top: 20px; }
.card { background: var(--panel); border: 1px solid var(--line); border-radius: 6px; padding: 12px 14px; }
.card b { display: block; font-size: 22px; font-variant-numeric: tabular-nums; }
.card span { color: var(--muted); font-size: 13px; }
table { width: 100%; border-collapse: collapse; font-size: 14px; }
th, td { text-align: left; vertical-align: top; padding: 7px 8px; border-bottom: 1px solid var(--line); }
th { color: var(--muted); font-weight: 600; font-size: 13px; }
td.num { font-variant-numeric: tabular-nums; white-space: nowrap; }
code, .mono { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 13px; }
.sev { font-weight: 600; white-space: nowrap; }
.error { color: var(--err); } .warning { color: var(--warn); } .info { color: var(--info); }
.help, .fix { color: var(--muted); margin-top: 4px; }
.fix::before { content: "fix: "; font-weight: 600; }
ul.next { margin: 0; padding-left: 18px; }
.idle { color: var(--muted); }
.wrap { overflow-x: auto; }
pre.mermaid { background: var(--panel); border: 1px solid var(--line); border-radius: 6px; padding: 12px; overflow-x: auto; }
"#;

/// The report page.
pub fn page(a: &Analysis, st: &Status) -> String {
    let mut h = String::new();
    let root = a.index.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| a.index.root.display().to_string());
    let count = |s: Severity| a.diagnostics.iter().filter(|d| d.severity == s).count();
    let fixable = a.diagnostics.iter().filter(|d| d.fix.as_ref().is_some_and(|f| f.safety == crate::diag::FixSafety::Safe)).count();
    let _ = write!(
        h,
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>rosfix report: {}</title><style>{STYLE}</style></head><body><main>\n",
        esc(&root)
    );
    let _ = writeln!(
        h,
        "<h1>rosfix report: {}</h1><p class=\"meta\">rosfix {} &middot; {} &middot; distribution {}</p>",
        esc(&root),
        env!("CARGO_PKG_VERSION"),
        esc(&a.index.root.display().to_string()),
        esc(st.distro.as_deref().unwrap_or("unknown"))
    );
    let cards = [
        (count(Severity::Error), "errors", "error"),
        (count(Severity::Warning), "warnings", "warning"),
        (count(Severity::Info), "notes", "info"),
        (fixable, "with a safe fix", ""),
        (st.packages, "packages", ""),
        (st.files_parsed, "files read", ""),
        (st.launch_graphs, "launch graphs", ""),
        (st.node_instances, "node instances", ""),
    ];
    h.push_str("<div class=\"cards\">");
    for (n, label, class) in cards {
        let _ = write!(h, "<div class=\"card\"><b class=\"{class}\">{n}</b><span>{label}</span></div>");
    }
    h.push_str("</div>\n");

    // ---- findings by file
    h.push_str("<h2>Findings</h2>\n");
    if a.diagnostics.is_empty() {
        h.push_str("<p>No findings.</p>\n");
    }
    let mut by_file: BTreeMap<&str, Vec<&crate::diag::Diagnostic>> = BTreeMap::new();
    for d in &a.diagnostics {
        by_file.entry(a.index.file(d.loc.file).rel_path.as_str()).or_default().push(d);
    }
    for (file, mut ds) in by_file {
        ds.sort_by_key(|d| d.loc.start);
        let _ = writeln!(h, "<h3>{}</h3><div class=\"wrap\"><table><tr><th>Line</th><th>Rule</th><th>Severity</th><th>Finding</th></tr>", esc(file));
        for d in ds {
            let line = a.index.file(d.loc.file).line_col(d.loc.start).0;
            let sev = d.severity.as_str();
            let summary = rules::find(d.rule).map_or("", |r| r.summary);
            let _ = write!(
                h,
                "<tr><td class=\"num\">{line}</td><td class=\"mono\" title=\"{}\">{}</td><td class=\"sev {sev}\">{sev}</td><td>{}",
                esc(summary),
                d.rule,
                esc(&d.message)
            );
            if let Some(help) = &d.help {
                let _ = write!(h, "<div class=\"help\">{}</div>", esc(help));
            }
            if let Some(f) = &d.fix {
                let _ = write!(h, "<div class=\"fix\">{}</div>", esc(&f.description));
            }
            h.push_str("</td></tr>\n");
        }
        h.push_str("</table></div>\n");
    }

    // ---- agents
    h.push_str("<h2>Agents</h2>\n<div class=\"wrap\"><table><tr><th>Agent</th><th>State</th><th>Findings</th><th>Checked</th><th>Next</th></tr>\n");
    for ag in &st.agents {
        let state = match ag.state {
            State::Ran => "ran",
            State::Reused => "reused",
            State::Idle => "idle",
            State::NotSelected => "not selected",
        };
        let class = if matches!(ag.state, State::Idle | State::NotSelected) { " class=\"idle\"" } else { "" };
        let findings: Vec<String> = ag.findings.iter().map(|(r, n)| format!("{r}&times;{n}")).collect();
        let _ = write!(
            h,
            "<tr{class}><td><b>{}</b><div class=\"help\">{}</div></td><td>{state}</td><td class=\"mono\">{}</td><td>{}</td><td>",
            esc(ag.id),
            esc(ag.title),
            if findings.is_empty() { "-".to_string() } else { findings.join(" ") },
            esc(&ag.checked.join(", "))
        );
        if !ag.next.is_empty() {
            h.push_str("<ul class=\"next\">");
            for n in &ag.next {
                let _ = write!(h, "<li>{}</li>", esc(n));
            }
            h.push_str("</ul>");
        }
        h.push_str("</td></tr>\n");
    }
    h.push_str("</table></div>\n");

    // ---- launch graph
    h.push_str("<h2>Launch graph</h2>\n");
    if st.launch_graphs == 0 {
        h.push_str("<p>No launch file was found.</p>\n");
    } else if st.node_instances > GRAPH_LIMIT {
        let _ = writeln!(
            h,
            "<p>{} node instances are too many to draw as one diagram. Use <code>rosfix graph --format dot</code> or <code>--format mermaid</code> for parts of the workspace.</p>",
            st.node_instances
        );
    } else {
        let _ = writeln!(
            h,
            "<p class=\"meta\">Boxes are node instances, rounded shapes topics, after namespaces and remaps.</p>\n<pre class=\"mermaid\">{}</pre>\n<script src=\"{MERMAID}\"></script>\n<script>if (window.mermaid) {{ mermaid.initialize({{ startOnLoad: true, securityLevel: \"strict\", theme: window.matchMedia(\"(prefers-color-scheme: dark)\").matches ? \"dark\" : \"default\" }}); }}</script>",
            esc(&crate::report::graph_mermaid(a))
        );
    }
    h.push_str("</main></body></html>\n");
    h
}

#[cfg(test)]
mod tests {
    #[test]
    fn escapes_markup() {
        assert_eq!(super::esc("<a href=\"x\">&</a>"), "&lt;a href=&quot;x&quot;&gt;&amp;&lt;/a&gt;");
    }
}
