//! Agents: one domain checker per problem area, dispatched on demand.
//!
//! Every agent declares
//! - **what it reads** ([`Agent::inputs`]): the file kinds whose facts it depends on, directly
//!   or through the launch graph;
//! - **when it has work** ([`Agent::relevant`]): a cheap check of the facts (e.g. the
//!   ros2_control agent has nothing to do without a `<ros2_control>` block).
//!
//! The dispatcher ([`run`]) wakes an agent only when it has work *and*, on a re-analysis,
//! when a file of a kind it reads changed (the change event). Agents whose inputs did not
//! change are not run: their previous findings are reused unchanged. Agents never write
//! files and never share mutable state, so skipping one cannot affect another.

use crate::config::Config;
use crate::diag::Diagnostic;
use crate::resolve::Graph;
use crate::workspace::{FileKind, Index};
use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

/// Every file kind (an agent that declares this reruns on any change).
pub const ALL_INPUTS: &[FileKind] = &[
    FileKind::Manifest,
    FileKind::CMakeLists,
    FileKind::SetupPy,
    FileKind::SetupCfg,
    FileKind::LaunchPy,
    FileKind::LaunchXml,
    FileKind::LaunchYaml,
    FileKind::ParamYaml,
    FileKind::Rviz,
    FileKind::Urdf,
    FileKind::Cpp,
    FileKind::Python,
    FileKind::Security,
    FileKind::BtXml,
    FileKind::MoveIt,
    FileKind::GzBridge,
    FileKind::Ros1,
    FileKind::Other,
];

/// Everything the launch graph (`resolve::build`) is computed from: packages and build
/// files, launch files, parameter files and node source code. Agents that read the graph
/// must list all of these.
pub const GRAPH_INPUTS: &[FileKind] = &[
    FileKind::Manifest,
    FileKind::CMakeLists,
    FileKind::SetupPy,
    FileKind::SetupCfg,
    FileKind::LaunchPy,
    FileKind::LaunchXml,
    FileKind::LaunchYaml,
    FileKind::ParamYaml,
    FileKind::Cpp,
    FileKind::Python,
    FileKind::Other,
];

pub mod control;
pub mod cross;
pub mod executor;
pub mod gzbridge;
pub mod launch;
pub mod lifecycle;
pub mod moveit;
pub mod nav2;
pub mod network;
pub mod packaging;
pub mod params;
pub mod qos;
pub mod ros1;
pub mod security;
pub mod simtime;
pub mod tf;
pub mod topics;
pub mod urdf;

/// Span of `value` written as a quoted string (or bare word) inside `within`, excluding quotes.
pub fn value_span(
    ix: &crate::workspace::Index,
    within: crate::source::Loc,
    value: &str,
) -> Option<crate::source::Loc> {
    if value.is_empty() {
        return None;
    }
    let f = ix.file(within.file);
    let text = f.slice(within);
    for q in ['"', '\''] {
        let needle = format!("{q}{value}{q}");
        if let Some(i) = text.find(&needle) {
            let s = within.start + i + 1;
            return Some(crate::source::Loc::new(within.file, s, s + value.len()));
        }
    }
    // Bare word (YAML lists, plain scalars).
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(i) = text[from..].find(value) {
        let s = from + i;
        let e = s + value.len();
        let ok_before = s == 0 || !(bytes[s - 1].is_ascii_alphanumeric() || bytes[s - 1] == b'_');
        let ok_after = e >= bytes.len() || !(bytes[e].is_ascii_alphanumeric() || bytes[e] == b'_');
        if ok_before && ok_after {
            return Some(crate::source::Loc::new(
                within.file,
                within.start + s,
                within.start + e,
            ));
        }
        from = e;
    }
    None
}

pub struct Context<'a> {
    pub ix: &'a Index,
    pub graph: &'a Graph,
    pub config: &'a Config,
}

pub trait Agent {
    /// Stable id used by `--agents` and in rule metadata.
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    /// File kinds this agent's findings depend on (directly or via the launch graph).
    /// A change to any other kind of file cannot change its findings. Over-declaring is
    /// safe (the agent just reruns more often); under-declaring is a bug, caught by the
    /// differential test in `tests/agents.rs`.
    fn inputs(&self) -> &'static [FileKind] {
        ALL_INPUTS
    }
    /// Cheap check: does the workspace contain anything this agent can analyse?
    fn relevant(&self, cx: &Context) -> bool;
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>);
}

/// A change event for a re-analysis: which kinds of files changed since `previous`.
pub struct ChangeEvent<'a> {
    pub changed: &'a BTreeSet<FileKind>,
    /// Findings of the previous analysis, per agent (before suppression and rule settings).
    pub previous: &'a HashMap<&'static str, Vec<Diagnostic>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentRun {
    pub id: &'static str,
    pub title: &'static str,
    /// Findings reused from the previous analysis because none of the agent's inputs changed.
    pub reused: bool,
    pub ran: bool,
    pub diagnostics: usize,
    pub micros: u128,
}

/// Reports files that could not be parsed (so nothing fails silently).
pub struct CoreAgent;

impl Agent for CoreAgent {
    fn id(&self) -> &'static str {
        "core"
    }
    fn title(&self) -> &'static str {
        "Parse errors"
    }
    fn relevant(&self, cx: &Context) -> bool {
        !cx.ix.parse_errors.is_empty()
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        for (f, e) in &cx.ix.parse_errors {
            let first_line: String = e.lines().next().unwrap_or("").chars().take(200).collect();
            out.push(Diagnostic::new(
                "R001",
                crate::source::Loc::new(*f, 0, 0),
                format!("could not parse this file: {first_line}"),
            ));
        }
    }
}

pub fn all_agents() -> Vec<Box<dyn Agent>> {
    vec![
        Box::new(CoreAgent),
        Box::new(params::ParamsAgent),
        Box::new(topics::TopicsAgent),
        Box::new(executor::ExecutorAgent),
        Box::new(control::ControlAgent),
        Box::new(tf::TfAgent),
        Box::new(lifecycle::LifecycleAgent),
        Box::new(launch::LaunchArgsAgent),
        Box::new(network::NetworkAgent),
        Box::new(packaging::PackagingAgent),
        Box::new(simtime::SimTimeAgent),
        Box::new(qos::QosAgent),
        Box::new(nav2::Nav2Agent),
        Box::new(moveit::MoveItAgent),
        Box::new(gzbridge::GzBridgeAgent),
        Box::new(urdf::UrdfAgent),
        Box::new(ros1::Ros1Agent),
        Box::new(security::SecurityAgent),
    ]
}

/// Dispatches the selected agents and records per-agent timing.
///
/// Without an event (first analysis) every selected agent with work runs. With an event,
/// an agent whose declared inputs are untouched by the change reuses its previous findings.
/// Returns all findings plus the findings per agent (for the next event).
pub fn run(
    cx: &Context,
    only: Option<&[String]>,
    event: Option<&ChangeEvent>,
) -> (
    Vec<Diagnostic>,
    Vec<AgentRun>,
    HashMap<&'static str, Vec<Diagnostic>>,
) {
    let mut all = Vec::new();
    let mut runs = Vec::new();
    let mut by_agent = HashMap::new();
    for a in all_agents() {
        let selected = only.is_none_or(|o| o.iter().any(|x| x == a.id()) || a.id() == "core");
        let t = Instant::now();
        let reuse = event.and_then(|e| {
            let touched = a.inputs().iter().any(|k| e.changed.contains(k));
            if touched {
                None
            } else {
                e.previous.get(a.id()).cloned()
            }
        });
        let reused = selected && reuse.is_some();
        let ran = selected && !reused && a.relevant(cx);
        let mut out = match (selected, reuse) {
            (true, Some(prev)) => prev,
            _ => Vec::new(),
        };
        if ran {
            a.run(cx, &mut out);
        }
        runs.push(AgentRun {
            id: a.id(),
            title: a.title(),
            reused,
            ran,
            diagnostics: out.len(),
            micros: t.elapsed().as_micros(),
        });
        if selected {
            by_agent.insert(a.id(), out.clone());
        }
        all.extend(out);
    }
    (all, runs, by_agent)
}
