//! Output formats: text, JSON and SARIF 2.1.0.

use crate::diag::{Diagnostic, FixSafety};
use crate::engine::{fingerprint, Analysis};
use crate::rules::{self, Severity, RULES};
use crate::source::Loc;
use serde_json::{json, Value};
use std::fmt::Write;

pub struct Style {
    pub color: bool,
}

impl Style {
    fn paint(&self, code: &str, s: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
}

fn sev_label(st: &Style, s: Severity) -> String {
    match s {
        Severity::Error => st.paint("1;31", "error"),
        Severity::Warning => st.paint("1;33", "warning"),
        Severity::Info => st.paint("1;36", "info"),
    }
}

pub fn text(a: &Analysis, st: &Style, verbose: bool) -> String {
    let mut out = String::new();
    let ix = &a.index;
    for d in &a.diagnostics {
        let f = ix.file(d.loc.file);
        let (line, col) = f.line_col(d.loc.start);
        let _ = writeln!(
            out,
            "{}[{}]: {}",
            sev_label(st, d.severity),
            st.paint("1", d.rule),
            d.message
        );
        let _ = writeln!(
            out,
            "  {} {}:{}:{}",
            st.paint("34", "-->"),
            f.rel_path,
            line,
            col
        );
        if d.loc.end > d.loc.start || d.loc.start > 0 {
            let src = f.line_text(line);
            if !src.trim().is_empty() {
                let shown: String = src.chars().take(160).collect();
                let _ = writeln!(
                    out,
                    "  {} {}",
                    st.paint("34", &format!("{line:>4} |")),
                    shown
                );
            }
        }
        for r in &d.related {
            let rf = ix.file(r.loc.file);
            let (rl, rc) = rf.line_col(r.loc.start);
            let _ = writeln!(
                out,
                "  {} {}:{}:{}: {}",
                st.paint("34", "note:"),
                rf.rel_path,
                rl,
                rc,
                r.message
            );
        }
        for (title, url) in &d.references {
            let _ = writeln!(out, "  {} {title} — {url}", st.paint("35", "see also:"));
        }
        if let Some(h) = &d.help {
            for (i, l) in h.lines().enumerate() {
                let _ = writeln!(
                    out,
                    "  {} {}",
                    st.paint("32", if i == 0 { "help:" } else { "     " }),
                    l
                );
            }
        }
        if let Some(fix) = &d.fix {
            let tag = match fix.safety {
                FixSafety::Safe => "fix (auto):",
                FixSafety::Assisted => "fix (assisted):",
                FixSafety::Suggestion => "fix (manual):",
            };
            let _ = writeln!(out, "  {} {}", st.paint("32", tag), fix.description);
        }
        if verbose {
            let _ = writeln!(
                out,
                "  {} {}",
                st.paint("90", "rule:"),
                d.rule_info().summary
            );
        }
        out.push('\n');
    }
    out
}

pub fn summary(a: &Analysis, st: &Style, hidden_by_baseline: usize) -> String {
    let count = |s: Severity| a.diagnostics.iter().filter(|d| d.severity == s).count();
    let (e, w, i) = (
        count(Severity::Error),
        count(Severity::Warning),
        count(Severity::Info),
    );
    let fixable = a
        .diagnostics
        .iter()
        .filter(|d| d.fix.as_ref().is_some_and(|f| f.safety == FixSafety::Safe))
        .count();
    let s = a.stats();
    let mut out = format!(
        "{} {} error(s), {} warning(s), {} info — {} packages, {} files analysed ({} launch graphs, {} nodes) in {} ms",
        st.paint("1", "rosfix:"),
        e,
        w,
        i,
        a.index.packages.len(),
        s.files_parsed,
        a.graph.roots.len(),
        a.graph.roots.iter().map(|r| r.nodes.len()).sum::<usize>(),
        a.timings.index_ms + a.timings.resolve_ms + a.timings.agents_ms
    );
    if fixable > 0 {
        let _ = write!(out, "\n{fixable} finding(s) have an automatic fix: run with --fix to preview, --fix --write to apply.");
    }
    if a.suppressed > 0 {
        let _ = write!(
            out,
            "\n{} finding(s) suppressed by `rosfix: ignore` comments.",
            a.suppressed
        );
    }
    if hidden_by_baseline > 0 {
        let _ = write!(
            out,
            "\n{hidden_by_baseline} known finding(s) hidden by the baseline."
        );
    }
    out
}

pub fn agents_table(a: &Analysis) -> String {
    let mut out = String::from("agent        ran   findings   time\n");
    for r in &a.runs {
        let _ = writeln!(
            out,
            "{:<12} {:<5} {:>8}   {:.1} ms",
            r.id,
            if r.ran {
                "yes"
            } else if r.reused {
                "reuse"
            } else {
                "skip"
            },
            r.diagnostics,
            r.micros as f64 / 1000.0
        );
    }
    let _ = writeln!(
        out,
        "index {} ms, resolve {} ms, agents {} ms; {} files seen, {} loaded ({} KiB), {} parsed, {} reused from cache",
        a.timings.index_ms,
        a.timings.resolve_ms,
        a.timings.agents_ms,
        a.stats().files_seen,
        a.stats().files_loaded,
        a.stats().bytes_loaded / 1024,
        a.stats().files_parsed,
        a.stats().files_reused
    );
    out
}

/// Human-readable dump of the resolved launch graph (what rosfix understood).
pub fn graph(a: &Analysis) -> String {
    use crate::resolve::{instance_endpoints, ImplStatus};
    let mut out = String::new();
    let (mut total, mut resolved, mut open, mut ext) = (0, 0, 0, 0);
    for r in &a.graph.roots {
        let _ = writeln!(
            out,
            "{}{}",
            a.index.file(r.root).rel_path,
            if r.partial { "  (partial)" } else { "" }
        );
        for n in &r.nodes {
            total += 1;
            let status = match n.impl_status {
                ImplStatus::Resolved => {
                    resolved += 1;
                    if n.params_open {
                        open += 1;
                    }
                    if n.params_open {
                        "source: linked (params open)"
                    } else if n.params_open_dotted {
                        "source: linked (plugin params open)"
                    } else {
                        "source: linked"
                    }
                }
                ImplStatus::External => {
                    ext += 1;
                    "source: external package"
                }
                ImplStatus::Unresolved => "source: not found",
            };
            let (eps, _) = instance_endpoints(&a.index, n);
            let bound = n
                .params
                .iter()
                .filter(|p| !crate::resolve::is_load_marker(p))
                .count();
            let _ = writeln!(
                out,
                "  {:<40} {}/{}  {}  params={}{}  endpoints={}{}",
                n.display_name(),
                n.pkg.as_deref().unwrap_or("?"),
                n.exec.as_deref().unwrap_or("?"),
                status,
                bound,
                if n.param_sources_complete { "" } else { "+?" },
                eps.len(),
                if n.conditional { "  [conditional]" } else { "" }
            );
        }
    }
    let _ = writeln!(out, "\n{} launch graphs, {total} node instances: {resolved} linked to source ({open} with open parameters), {ext} external", a.graph.roots.len());
    out
}

/// topic → (message type, publisher node indices, subscriber node indices)
type TopicLinks = std::collections::BTreeMap<String, (Option<String>, Vec<usize>, Vec<usize>)>;

/// One launch graph reduced to what a picture needs: node instances and the topics that
/// connect them (after namespaces and remaps).
struct GraphView {
    file: String,
    partial: bool,
    /// (name, package/executable, source status, conditional)
    nodes: Vec<(String, String, &'static str, bool)>,
    topics: TopicLinks,
}

fn graph_views(a: &Analysis) -> Vec<GraphView> {
    use crate::facts::EndpointKind;
    use crate::resolve::{instance_endpoints, ImplStatus};
    a.graph
        .roots
        .iter()
        .map(|r| {
            let mut v = GraphView {
                file: a.index.file(r.root).rel_path.clone(),
                partial: r.partial,
                nodes: Vec::new(),
                topics: Default::default(),
            };
            for (i, n) in r.nodes.iter().enumerate() {
                let status = match n.impl_status {
                    ImplStatus::Resolved => "linked",
                    ImplStatus::External => "external",
                    ImplStatus::Unresolved => "not_found",
                };
                v.nodes.push((
                    n.display_name(),
                    format!("{}/{}", n.pkg.as_deref().unwrap_or("?"), n.exec.as_deref().unwrap_or("?")),
                    status,
                    n.conditional,
                ));
                for e in instance_endpoints(&a.index, n).0 {
                    if !e.kind.is_topic() {
                        continue;
                    }
                    let t = v.topics.entry(e.name.clone()).or_default();
                    if t.0.is_none() {
                        t.0 = e.msg_type.clone();
                    }
                    let side = if e.kind == EndpointKind::Publisher { &mut t.1 } else { &mut t.2 };
                    if !side.contains(&i) {
                        side.push(i);
                    }
                }
            }
            v
        })
        .collect()
}

/// The resolved launch graphs as JSON: nodes and the topics connecting them.
pub fn graph_json(a: &Analysis) -> String {
    let roots: Vec<Value> = graph_views(a)
        .into_iter()
        .map(|v| {
            json!({
                "file": v.file,
                "partial": v.partial,
                "nodes": v.nodes.iter().enumerate().map(|(i, (name, exe, status, cond))| json!({
                    "id": i, "name": name, "executable": exe, "source": status, "conditional": cond,
                })).collect::<Vec<_>>(),
                "topics": v.topics.iter().map(|(name, (ty, p, s))| json!({
                    "name": name, "type": ty, "publishers": p, "subscribers": s,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::to_string_pretty(&json!({ "graphs": roots })).unwrap_or_default() + "\n"
}

/// Findings in the Reviewdog Diagnostic Format (rdjson). Fixes that passed verification
/// (`verified`, from `fix::run`) and edit only the finding's own file become `suggestions`,
/// which reviewdog's github-pr-review reporter posts as one-click suggested changes. Paths
/// are relative to the current directory, so reviewdog can match them to the diff.
pub fn rdjson(a: &Analysis, verified: Option<&crate::fix::FixReport>) -> String {
    let cwd = std::env::current_dir().ok();
    let path_of = |file: crate::source::FileId| {
        let f = a.index.file(file);
        cwd.as_ref()
            .and_then(|c| f.abs_path.strip_prefix(c).ok())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| f.rel_path.clone())
    };
    // 1-based line and 1-based UTF-8 byte column, as rdjson specifies.
    let pos = |file: crate::source::FileId, byte: usize| {
        let t = &a.index.file(file).text;
        let byte = byte.min(t.len());
        let line_start = t[..byte].rfind('\n').map_or(0, |i| i + 1);
        json!({"line": t[..byte].matches('\n').count() + 1, "column": byte - line_start + 1})
    };
    let range = |file, s: usize, e: usize| json!({"start": pos(file, s), "end": pos(file, e.max(s))});
    let diagnostics: Vec<Value> = a
        .diagnostics
        .iter()
        .filter(|d| !a.index.file(d.loc.file).abs_path.as_os_str().is_empty())
        .map(|d| {
            let severity = match d.severity {
                Severity::Error => "ERROR",
                Severity::Warning => "WARNING",
                Severity::Info => "INFO",
            };
            let mut message = d.message.clone();
            if let Some(h) = &d.help {
                message.push_str("\n\n");
                message.push_str(h);
            }
            let suggestions: Vec<Value> = d
                .fix
                .as_ref()
                .filter(|f| f.edits.iter().all(|e| e.file == d.loc.file))
                .filter(|f| verified.is_some_and(|v| v.applied.iter().any(|p| p.rule == d.rule && p.message == d.message && p.edits == f.edits)))
                .map(|f| f.edits.iter().map(|e| json!({"range": range(e.file, e.start, e.end), "text": e.replacement})).collect())
                .unwrap_or_default();
            let mut v = json!({
                "message": message,
                "location": {"path": path_of(d.loc.file), "range": range(d.loc.file, d.loc.start, d.loc.end)},
                "severity": severity,
                "code": {"value": d.rule},
            });
            if !suggestions.is_empty() {
                v["suggestions"] = Value::Array(suggestions);
            }
            v
        })
        .collect();
    serde_json::to_string_pretty(&json!({"source": {"name": "rosfix"}, "diagnostics": diagnostics})).unwrap_or_default() + "\n"
}

/// The static callback graph: per launched node, its executor, callback groups, callbacks
/// (timers with their periods, subscriptions with their resolved topics) and publishers,
/// plus the publisher-to-callback edges between nodes. Input for response-time analysis
/// or for CARET path definitions; everything here is read from source, nothing is measured.
pub fn graph_callbacks(a: &Analysis) -> String {
    use crate::facts::{CallbackKind, EndpointKind, ExecutorKind, GroupKind};
    use crate::resolve::{instance_endpoints, ImplStatus};
    let at = |loc: Loc| {
        let f = a.index.file(loc.file);
        format!("{}:{}", f.rel_path, f.line_col(loc.start).0)
    };
    let graphs: Vec<Value> = a
        .graph
        .roots
        .iter()
        .map(|r| {
            let mut nodes = Vec::new();
            // topic -> (publishing node ids), and (topic, callback id) for subscriptions.
            let mut publishers: std::collections::BTreeMap<String, Vec<String>> = Default::default();
            let mut subscribers: Vec<(String, String)> = Vec::new();
            for (i, n) in r.nodes.iter().enumerate() {
                let id = format!("n{i}");
                let (eps, _) = instance_endpoints(&a.index, n);
                let mut callbacks = Vec::new();
                let mut groups = Vec::new();
                let mut executors: Vec<&str> = Vec::new();
                for (f, k) in &n.impls {
                    let code = &a.index.code[f];
                    let imp = &code.nodes[*k];
                    for e in &code.executors {
                        let kind = match e.kind {
                            ExecutorKind::SingleThreaded => "single_threaded",
                            ExecutorKind::MultiThreaded => "multi_threaded",
                            ExecutorKind::Unknown => "unknown",
                        };
                        if !executors.contains(&kind) {
                            executors.push(kind);
                        }
                    }
                    for g in &imp.groups {
                        groups.push(json!({
                            "name": g.var,
                            "kind": if g.kind == GroupKind::Reentrant { "reentrant" } else { "mutually_exclusive" },
                            "dedicated_executor": imp.dedicated_groups.contains(&g.var),
                            "source": at(g.loc),
                        }));
                    }
                    for c in &imp.callbacks {
                        let cid = format!("{id}.c{}", callbacks.len());
                        let kind = match c.kind {
                            CallbackKind::Timer => "timer",
                            CallbackKind::Subscription => "subscription",
                            CallbackKind::Service => "service",
                            CallbackKind::Action => "action",
                        };
                        let ep = eps.iter().find(|e| e.loc == c.loc && e.kind != EndpointKind::Publisher);
                        if let (CallbackKind::Subscription, Some(e)) = (c.kind, ep) {
                            subscribers.push((e.name.clone(), cid.clone()));
                        }
                        callbacks.push(json!({
                            "id": cid,
                            "kind": kind,
                            "period_ms": c.period_ms,
                            "topic": ep.map(|e| e.name.clone()),
                            "type": ep.and_then(|e| e.msg_type.clone()),
                            "group": c.group.clone().unwrap_or_else(|| "default".to_string()),
                            "source": at(c.loc),
                        }));
                    }
                }
                let pubs: Vec<Value> = eps
                    .iter()
                    .filter(|e| e.kind == EndpointKind::Publisher)
                    .map(|e| {
                        let list = publishers.entry(e.name.clone()).or_default();
                        if !list.contains(&id) {
                            list.push(id.clone());
                        }
                        json!({ "topic": e.name, "type": e.msg_type, "source": at(e.loc) })
                    })
                    .collect();
                nodes.push(json!({
                    "id": id,
                    "name": n.display_name(),
                    "executable": format!("{}/{}", n.pkg.as_deref().unwrap_or("?"), n.exec.as_deref().unwrap_or("?")),
                    "source": match n.impl_status {
                        ImplStatus::Resolved => "linked",
                        ImplStatus::External => "external",
                        ImplStatus::Unresolved => "not_found",
                    },
                    "conditional": n.conditional,
                    "executors": executors,
                    "callback_groups": groups,
                    "callbacks": callbacks,
                    "publishers": pubs,
                }));
            }
            let edges: Vec<Value> = subscribers
                .iter()
                .flat_map(|(topic, cb)| {
                    publishers.get(topic).into_iter().flatten().map(move |p| json!({ "from_node": p, "topic": topic, "to_callback": cb }))
                })
                .collect();
            json!({
                "file": a.index.file(r.root).rel_path,
                "partial": r.partial,
                "nodes": nodes,
                "edges": edges,
            })
        })
        .collect();
    serde_json::to_string_pretty(&json!({ "format": "rosfix-callback-graph", "version": 1, "graphs": graphs })).unwrap_or_default() + "\n"
}

/// The resolved launch graphs in Graphviz DOT: one cluster per launch file, nodes as
/// boxes, topics as ellipses, edges from publishers and to subscribers.
pub fn graph_dot(a: &Analysis) -> String {
    // A quoted DOT label; `\n` between the escaped lines is DOT's line break.
    let q = |lines: &[&str]| {
        let esc: Vec<String> = lines.iter().map(|s| s.replace('\\', "\\\\").replace('"', "\\\"")).collect();
        format!("\"{}\"", esc.join("\\n"))
    };
    let mut out = String::from("digraph rosfix {\n  rankdir=LR;\n  node [fontname=\"Helvetica\", fontsize=10];\n");
    for (g, v) in graph_views(a).iter().enumerate() {
        let title = format!("{}{}", v.file, if v.partial { " (partial)" } else { "" });
        let _ = writeln!(out, "  subgraph cluster_{g} {{\n    label={};", q(&[&title]));
        for (i, (name, exe, status, cond)) in v.nodes.iter().enumerate() {
            let style = match (*status, *cond) {
                (_, true) => ", style=dashed",
                ("not_found", _) => ", color=red",
                _ => "",
            };
            let label = if name == exe { q(&[name]) } else { q(&[name, exe]) };
            let _ = writeln!(out, "    g{g}n{i} [shape=box, label={label}{style}];");
        }
        for (t, (name, (ty, pubs, subs))) in v.topics.iter().enumerate() {
            let label = match ty {
                Some(ty) => q(&[name, ty]),
                None => q(&[name]),
            };
            let _ = writeln!(out, "    g{g}t{t} [shape=ellipse, label={label}];");
            for p in pubs {
                let _ = writeln!(out, "    g{g}n{p} -> g{g}t{t};");
            }
            for s in subs {
                let _ = writeln!(out, "    g{g}t{t} -> g{g}n{s};");
            }
        }
        out.push_str("  }\n");
    }
    out.push_str("}\n");
    out
}

/// The resolved launch graphs as a Mermaid flowchart (renders on GitHub and in most
/// Markdown viewers).
pub fn graph_mermaid(a: &Analysis) -> String {
    let q = |s: &str| format!("\"{}\"", s.replace('"', "#quot;"));
    let mut out = String::from("flowchart LR\n");
    for (g, v) in graph_views(a).iter().enumerate() {
        let _ = writeln!(out, "  subgraph g{g}[{}]", q(&format!("{}{}", v.file, if v.partial { " (partial)" } else { "" })));
        for (i, (name, exe, status, cond)) in v.nodes.iter().enumerate() {
            let note = match (*status, *cond) {
                (_, true) => " (conditional)",
                ("not_found", _) => " (source not found)",
                _ => "",
            };
            let label = if name == exe { format!("{name}{note}") } else { format!("{name}<br/>{exe}{note}") };
            let _ = writeln!(out, "    g{g}n{i}[{}]", q(&label));
        }
        for (t, (name, (_, pubs, subs))) in v.topics.iter().enumerate() {
            let _ = writeln!(out, "    g{g}t{t}([{}])", q(name));
            for p in pubs {
                let _ = writeln!(out, "    g{g}n{p} --> g{g}t{t}");
            }
            for s in subs {
                let _ = writeln!(out, "    g{g}t{t} --> g{g}n{s}");
            }
        }
        out.push_str("  end\n");
    }
    out
}

fn loc_json(a: &Analysis, loc: Loc) -> Value {
    let f = a.index.file(loc.file);
    let (sl, sc) = f.line_col(loc.start);
    let (el, ec) = f.line_col(loc.end);
    json!({ "file": f.rel_path, "line": sl, "column": sc, "end_line": el, "end_column": ec, "start_byte": loc.start, "end_byte": loc.end })
}

pub fn json(a: &Analysis) -> String {
    let items: Vec<Value> = a
        .diagnostics
        .iter()
        .map(|d| {
            json!({
                "rule": d.rule,
                "name": d.rule_info().name,
                "agent": d.rule_info().agent,
                "severity": d.severity.as_str(),
                "message": d.message,
                "location": loc_json(a, d.loc),
                "related": d.related.iter().map(|r| json!({"message": r.message, "location": loc_json(a, r.loc)})).collect::<Vec<_>>(),
                "help": d.help,
                "references": d.references.iter().map(|(t, u)| json!({"title": t, "url": u})).collect::<Vec<_>>(),
                "fix": d.fix.as_ref().map(|f| json!({
                    "description": f.description,
                    "safety": f.safety,
                    "edits": f.edits.iter().map(|e| json!({"file": a.index.file(e.file).rel_path, "start_byte": e.start, "end_byte": e.end, "replacement": e.replacement})).collect::<Vec<_>>(),
                })),
                "fingerprint": fingerprint(&a.index, d),
            })
        })
        .collect();
    serde_json::to_string_pretty(&json!({
        "version": 1,
        "tool": {"name": "rosfix", "version": env!("CARGO_PKG_VERSION")},
        "diagnostics": items,
        "agents": a.runs,
        "timings_ms": a.timings,
    }))
    .unwrap_or_default()
}

fn sarif_region(a: &Analysis, loc: Loc) -> Value {
    let f = a.index.file(loc.file);
    let (sl, sc) = f.line_col(loc.start);
    let (el, ec) = f.line_col(loc.end.max(loc.start));
    json!({"startLine": sl, "startColumn": sc, "endLine": el, "endColumn": ec.max(if el == sl { sc } else { 1 })})
}

fn sarif_location(a: &Analysis, loc: Loc) -> Value {
    json!({
        "physicalLocation": {
            "artifactLocation": {"uri": a.index.file(loc.file).rel_path, "uriBaseId": "%SRCROOT%"},
            "region": sarif_region(a, loc),
        }
    })
}

fn sarif_level(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "note",
    }
}

pub fn sarif(a: &Analysis) -> String {
    let rules: Vec<Value> = RULES
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "name": r.name,
                "shortDescription": {"text": r.summary},
                "fullDescription": {"text": r.explanation},
                "help": {"text": r.explanation},
                "defaultConfiguration": {"level": sarif_level(r.severity), "enabled": r.enabled_by_default},
                "properties": {"agent": r.agent},
            })
        })
        .collect();
    let results: Vec<Value> = a
        .diagnostics
        .iter()
        .map(|d: &Diagnostic| {
            let idx = RULES.iter().position(|r| r.id == d.rule).unwrap_or(0);
            let mut msg = d.message.clone();
            if let Some(h) = &d.help {
                msg = format!("{msg}\n{h}");
            }
            for (t, u) in &d.references {
                msg = format!("{msg}\nSee also: {t} ({u})");
            }
            let mut r = json!({
                "ruleId": d.rule,
                "ruleIndex": idx,
                "level": sarif_level(d.severity),
                "message": {"text": msg},
                "locations": [sarif_location(a, d.loc)],
                "partialFingerprints": {"rosfix/v1": fingerprint(&a.index, d)},
            });
            if !d.related.is_empty() {
                r["relatedLocations"] = Value::Array(
                    d.related
                        .iter()
                        .enumerate()
                        .map(|(i, rel)| {
                            let mut l = sarif_location(a, rel.loc);
                            l["id"] = json!(i + 1);
                            l["message"] = json!({"text": rel.message});
                            l
                        })
                        .collect(),
                );
            }
            if let Some(f) = &d.fix {
                r["fixes"] = json!([{
                    "description": {"text": f.description},
                    "artifactChanges": f.edits.iter().map(|e| json!({
                        "artifactLocation": {"uri": a.index.file(e.file).rel_path, "uriBaseId": "%SRCROOT%"},
                        "replacements": [{
                            "deletedRegion": sarif_region(a, Loc::new(e.file, e.start, e.end)),
                            "insertedContent": {"text": e.replacement},
                        }],
                    })).collect::<Vec<_>>(),
                }]);
            }
            r
        })
        .collect();
    let _ = rules::find;
    serde_json::to_string_pretty(&json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {
                "name": "rosfix",
                "version": env!("CARGO_PKG_VERSION"),
                "informationUri": "https://github.com/SohamWalam11/ros2-safety-lint",
                "rules": rules,
            }},
            "originalUriBaseIds": {"%SRCROOT%": {"uri": "file:///"}},
            "results": results,
        }],
    }))
    .unwrap_or_default()
}
