//! `rosfix status`: what each agent did on this workspace, and what would let it do more.
//!
//! For every agent: whether it ran (or was skipped, reused, or not selected), the files and
//! facts it worked on, its findings per rule, and the next actions: the concrete things that
//! currently limit it (no ROS distribution, installed nodes nobody describes, launch files with
//! dynamic parts, QoS that is not known statically) and the fixes that are ready to apply.

use crate::agents::all_agents;
use crate::diag::FixSafety;
use crate::engine::Analysis;
use crate::resolve::{instance_endpoints, ImplStatus};
use crate::rules::Severity;
use crate::workspace::FileKind;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Analysed the workspace in this run.
    Ran,
    /// Its inputs did not change since the previous analysis; findings were reused.
    Reused,
    /// Nothing in the workspace for it to check.
    Idle,
    /// Left out with `--agents`.
    NotSelected,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentStatus {
    pub id: &'static str,
    pub title: &'static str,
    pub state: State,
    pub micros: u128,
    /// File kind → number of workspace files of that kind the agent reads.
    pub inputs: BTreeMap<String, usize>,
    /// What it worked on, as short facts ("41 parameter files", …).
    pub checked: Vec<String>,
    /// Rule id → findings.
    pub findings: BTreeMap<&'static str, usize>,
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
    /// Findings of this agent with an automatic (safe) fix.
    pub fixable: usize,
    /// What limits the agent now and what would let it check more; ready fixes.
    pub next: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub distro: Option<String>,
    pub installed_prefixes: usize,
    pub packages: usize,
    pub files_parsed: usize,
    pub launch_graphs: usize,
    pub node_instances: usize,
    pub agents: Vec<AgentStatus>,
}

fn kind_name(k: FileKind) -> &'static str {
    match k {
        FileKind::Manifest => "package.xml",
        FileKind::CMakeLists => "CMakeLists.txt",
        FileKind::SetupPy => "setup.py",
        FileKind::SetupCfg => "setup.cfg",
        FileKind::LaunchPy => "Python launch",
        FileKind::LaunchXml => "XML launch",
        FileKind::LaunchYaml => "YAML launch",
        FileKind::ParamYaml => "parameter YAML",
        FileKind::Rviz => "RViz config",
        FileKind::Urdf => "URDF/xacro",
        FileKind::Cpp => "C++",
        FileKind::Python => "Python",
        FileKind::Security => "SROS2",
        FileKind::BtXml => "behavior tree XML",
        FileKind::MoveIt => "MoveIt config",
        FileKind::GzBridge => "ros_gz_bridge config",
        FileKind::Ros1 => "ROS 1 source",
        FileKind::Other => "other",
    }
}

/// Facts about the launch graphs that limit several agents.
struct Limits {
    external_unknown: BTreeMap<String, usize>,
    not_found: usize,
    params_open: usize,
    partial_graphs: usize,
    endpoints: usize,
    endpoints_incomplete_nodes: usize,
    qos_unknown: usize,
    nodes_with_params: usize,
}

fn limits(a: &Analysis) -> Limits {
    let mut l = Limits {
        external_unknown: BTreeMap::new(),
        not_found: 0,
        params_open: 0,
        partial_graphs: a.graph.roots.iter().filter(|r| r.partial).count(),
        endpoints: 0,
        endpoints_incomplete_nodes: 0,
        qos_unknown: 0,
        nodes_with_params: 0,
    };
    for (_, _, n) in a.graph.instances() {
        match n.impl_status {
            ImplStatus::External if n.impls.is_empty() => {
                *l.external_unknown.entry(n.pkg.clone().unwrap_or_else(|| "?".into())).or_default() += 1;
            }
            ImplStatus::Unresolved => l.not_found += 1,
            _ => {}
        }
        if n.params_open {
            l.params_open += 1;
        }
        if n.params.iter().any(|p| !crate::resolve::is_load_marker(p)) {
            l.nodes_with_params += 1;
        }
        if n.impl_status == ImplStatus::Resolved || !n.impls.is_empty() {
            let (eps, complete) = instance_endpoints(&a.index, n);
            if !complete {
                l.endpoints_incomplete_nodes += 1;
            }
            for e in eps.iter().filter(|e| e.kind.is_topic()) {
                l.endpoints += 1;
                if e.qos.reliability.is_none() {
                    l.qos_unknown += 1;
                }
            }
        }
    }
    l
}

pub fn status(a: &Analysis) -> Status {
    let ix = &a.index;
    let l = limits(a);
    let distro = ix.installed.distro.clone();
    let mut by_kind: BTreeMap<FileKind, usize> = BTreeMap::new();
    for (i, k) in ix.kinds.iter().enumerate() {
        if !ix.external_files.contains(&i) {
            *by_kind.entry(*k).or_default() += 1;
        }
    }
    let ran: BTreeMap<&str, &crate::agents::AgentRun> = a.runs.iter().map(|r| (r.id, r)).collect();
    let selected = |id: &str| a.runs.iter().any(|r| r.id == id);
    let mut agents = Vec::new();
    for ag in all_agents() {
        let id = ag.id();
        let run = ran.get(id);
        let state = match run {
            // core reads every file; "nothing reported" still means it ran.
            _ if id == "core" => State::Ran,
            None if !selected(id) => State::NotSelected,
            None => State::Idle,
            Some(r) if r.reused => State::Reused,
            Some(r) if r.ran => State::Ran,
            Some(_) => State::Idle,
        };
        let inputs: BTreeMap<String, usize> = ag
            .inputs()
            .iter()
            .filter_map(|k| by_kind.get(k).map(|n| (kind_name(*k).to_string(), *n)))
            .filter(|(_, n)| *n > 0)
            .collect();
        let mut s = AgentStatus {
            id,
            title: ag.title(),
            state,
            micros: run.map_or(0, |r| r.micros),
            inputs,
            checked: Vec::new(),
            findings: BTreeMap::new(),
            errors: 0,
            warnings: 0,
            infos: 0,
            fixable: 0,
            next: Vec::new(),
        };
        for d in a.diagnostics.iter().filter(|d| d.rule_info().agent == id) {
            *s.findings.entry(d.rule).or_default() += 1;
            match d.severity {
                Severity::Error => s.errors += 1,
                Severity::Warning => s.warnings += 1,
                Severity::Info => s.infos += 1,
            }
            if d.fix.as_ref().is_some_and(|f| f.safety == FixSafety::Safe) {
                s.fixable += 1;
            }
        }
        describe(a, &l, distro.as_deref(), &mut s);
        agents.push(s);
    }
    Status {
        distro,
        installed_prefixes: ix.installed.prefixes.len(),
        packages: ix.packages.len(),
        files_parsed: ix.stats.files_parsed,
        launch_graphs: a.graph.roots.len(),
        node_instances: a.graph.instances().count(),
        agents,
    }
}

/// Agent-specific facts and next actions.
fn describe(a: &Analysis, l: &Limits, distro: Option<&str>, s: &mut AgentStatus) {
    let ix = &a.index;
    let n = |v: usize, what: &str| format!("{v} {what}");
    let external: usize = l.external_unknown.values().sum();
    let ext_names = || {
        let v: Vec<String> = l.external_unknown.iter().take(6).map(|(p, c)| if *c > 1 { format!("{p} ×{c}") } else { p.clone() }).collect();
        let more = l.external_unknown.len().saturating_sub(6);
        format!("{}{}", v.join(", "), if more > 0 { format!(", and {more} more") } else { String::new() })
    };
    match s.id {
        "core" => s.checked.push(n(ix.stats.files_parsed, "files parsed")),
        "params" => {
            s.checked.push(n(ix.param_files.len(), "parameter files"));
            s.checked.push(n(l.nodes_with_params, "launched nodes with parameters bound"));
            if l.params_open > 0 {
                s.next.push(format!("{} node(s) accept parameters rosfix cannot enumerate (plugins, dynamic names): only their top-level names are checked", l.params_open));
            }
        }
        "topics" | "qos" => {
            s.checked.push(n(l.endpoints, "topic endpoints in the launch graphs"));
            if l.endpoints_incomplete_nodes > 0 {
                s.next.push(format!("{} node(s) have an interface rosfix could only partly determine (topic names computed at runtime, plugins, or no statically known node name): checks that need every endpoint of a topic stay silent there", l.endpoints_incomplete_nodes));
            }
            if s.id == "qos" && l.qos_unknown > 0 {
                s.next.push(format!("{} endpoint(s) have a QoS not known statically (variables, system defaults without a Fast DDS profile); `rosfix check-bag` can compare with a recording", l.qos_unknown));
            }
        }
        "executor" => s.checked.push(n(ix.code.len(), "C++/Python source files with ROS code")),
        "control" | "tf" => s.checked.push(n(ix.robots.len(), "robot descriptions")),
        "launch" | "lifecycle" | "simtime" | "network" => s.checked.push(n(a.graph.roots.len(), "launch graphs")),
        "packaging" => s.checked.push(n(ix.packages.len(), "packages")),
        _ => {}
    }
    if matches!(s.id, "launch" | "topics" | "simtime" | "lifecycle" | "tf") && l.partial_graphs > 0 {
        s.next.push(format!("{} launch graph(s) have dynamic parts (OpaqueFunction, unresolved includes or namespaces): checks that need the whole graph are off there; `rosfix graph` shows which", l.partial_graphs));
    }
    if matches!(s.id, "params" | "topics" | "qos" | "nav2") {
        if distro.is_none() {
            s.next.push("no ROS distribution is set: knowledge packs are off, so nodes from installed packages are not checked; pass --distro, set `distro` in rosfix.toml, or source a ROS environment".into());
        }
        if external > 0 {
            s.next.push(format!(
                "{external} launched node(s) from installed packages have no knowledge-pack entry or NoDL document, so their parameters and endpoints are unknown: {}",
                ext_names()
            ));
        }
    }
    if s.id == "packaging" && l.not_found > 0 {
        s.next.push(format!("{} launched node(s) were not found in the workspace or the installed environment; source the workspace's install space or pass --ros-prefix", l.not_found));
    }
    if s.fixable > 0 {
        s.next.push(format!("{} finding(s) have verified automatic fixes: `rosfix fix` shows them, `rosfix fix --write` applies them", s.fixable));
    }
    let assistable = a
        .diagnostics
        .iter()
        .filter(|d| d.rule_info().agent == s.id && d.repair.is_some() && d.fix.is_none())
        .count();
    if assistable > 0 {
        s.next.push(format!("{assistable} finding(s) have a closed set of valid repairs: `rosfix fix --assist` proposes one"));
    }
    match s.state {
        State::Idle => {
            let wants: Vec<String> = crate::agents::all_agents()
                .into_iter()
                .find(|g| g.id() == s.id)
                .map(|g| g.inputs().iter().map(|k| kind_name(*k).to_string()).collect())
                .unwrap_or_default();
            if !wants.is_empty() && wants.len() < 6 {
                s.next.insert(0, format!("idle: nothing to check; it starts when the workspace has {}", wants.join(", ")));
            } else {
                s.next.insert(0, "idle: nothing in this workspace for it to check".into());
            }
        }
        State::NotSelected => s.next.insert(0, "not selected with --agents".into()),
        _ => {}
    }
}

pub fn text(st: &Status) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "rosfix status: {} package(s), {} file(s) parsed, {} launch graph(s), {} node instance(s)",
        st.packages, st.files_parsed, st.launch_graphs, st.node_instances
    );
    let _ = writeln!(
        s,
        "ROS distribution: {}; installed prefixes read: {}\n",
        st.distro.as_deref().unwrap_or("not set (knowledge packs off)"),
        st.installed_prefixes
    );
    for a in &st.agents {
        let state = match a.state {
            State::Ran => format!("ran in {:.1} ms", a.micros as f64 / 1000.0),
            State::Reused => "reused (inputs unchanged)".into(),
            State::Idle => "idle".into(),
            State::NotSelected => "not selected".into(),
        };
        let _ = writeln!(s, "{} - {} [{state}]", a.id, a.title);
        if !a.inputs.is_empty() {
            let v: Vec<String> = a.inputs.iter().map(|(k, n)| format!("{n} {k}")).collect();
            let _ = writeln!(s, "  reads:     {}", v.join(", "));
        }
        if !a.checked.is_empty() {
            let _ = writeln!(s, "  checked:   {}", a.checked.join("; "));
        }
        if a.findings.is_empty() {
            if a.state == State::Ran || a.state == State::Reused {
                let _ = writeln!(s, "  findings:  none");
            }
        } else {
            let v: Vec<String> = a.findings.iter().map(|(r, n)| format!("{r} ×{n}")).collect();
            let _ = writeln!(
                s,
                "  findings:  {} ({} error, {} warning, {} info)",
                v.join(", "),
                a.errors,
                a.warnings,
                a.infos
            );
        }
        for n in &a.next {
            let _ = writeln!(s, "  next:      {n}");
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::engine::{analyze, Options};

    #[test]
    fn status_reports_every_agent_and_its_limits() {
        let dir = tempfile::tempdir().unwrap();
        for (rel, text) in [
            ("p/package.xml", "<package format=\"3\"><name>p</name></package>"),
            ("p/CMakeLists.txt", "project(p)\nadd_executable(n src/n.cpp)\ninstall(TARGETS n DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch config DESTINATION share/${PROJECT_NAME})\n"),
            ("p/src/n.cpp", "class N : public rclcpp::Node { public: N() : Node(\"n\") { declare_parameter(\"rate\", 1.0); } };\n"),
            ("p/config/n.yaml", "n:\n  ros__parameters:\n    rate: 2\n"),
            ("p/launch/a.launch.xml", "<launch><node pkg=\"p\" exec=\"n\"><param from=\"$(find-pkg-share p)/config/n.yaml\"/></node><node pkg=\"nav2_amcl\" exec=\"amcl\"/></launch>"),
        ] {
            let f = dir.path().join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        }
        let a = analyze(dir.path(), &Config::default(), &Options::default());
        let st = super::status(&a);
        assert_eq!(st.agents.len(), crate::agents::all_agents().len());
        let params = st.agents.iter().find(|x| x.id == "params").unwrap();
        assert_eq!(params.state, super::State::Ran);
        assert_eq!(params.findings.get("P004"), Some(&1));
        assert_eq!(params.fixable, 1);
        let text = super::text(&st);
        assert!(text.contains("no ROS distribution is set"), "{text}");
        assert!(text.contains("nav2_amcl"), "{text}");
        assert!(text.contains("`rosfix fix --write` applies them"), "{text}");
        let security = st.agents.iter().find(|x| x.id == "security").unwrap();
        assert_eq!(security.state, super::State::Idle);
    }
}
