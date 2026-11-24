//! Lifecycle agent: managed (lifecycle) nodes and the `lifecycle_manager` that drives them.
//!
//! A lifecycle node does nothing until something configures and activates it. In Nav2-style
//! launch files that is a `lifecycle_manager` with a `node_names` list. Two silent failures:
//! - M001 a lifecycle node is launched next to a manager that does not list it: it stays
//!   unconfigured (no topics, no services) and nothing reports it;
//! - M002 `node_names` lists a node that is not launched: bringup stops at that name.
//!
//! Only graphs whose managers' `node_names` are fully resolved, and whose nodes all have
//! known names, are checked.

use super::{value_span, Agent, Context};
use crate::diag::Diagnostic;
use crate::facts::{LaunchNodeKind, ParamValue};
use crate::resolve::{join_ns, NodeInstance, RootGraph};
use crate::util::closest;
use std::collections::BTreeSet;

pub struct LifecycleAgent;

fn is_manager(n: &NodeInstance) -> bool {
    n.exec.as_deref() == Some("lifecycle_manager")
}

/// Launched as a lifecycle node, or implemented by a class deriving from a lifecycle node.
fn is_lifecycle(cx: &Context, n: &NodeInstance) -> bool {
    n.decl.kind == LaunchNodeKind::LifecycleNode
        || n.impls.iter().any(|(f, i)| {
            cx.ix.code[f].nodes[*i].base.as_deref().is_some_and(|b| {
                b.contains("LifecycleNode")
                    || b.contains("lifecycle.Node")
                    || b.contains("lifecycle::Node")
            })
        })
}

/// Fully-qualified name: from the launch file, else the name the source code gives the node
/// (`Node("map_saver")`) when its implementation is unambiguous.
fn effective_fq(cx: &Context, n: &NodeInstance) -> Option<String> {
    if let Some(f) = &n.fq_name {
        return Some(f.clone());
    }
    let names: BTreeSet<&str> = n
        .impls
        .iter()
        .filter_map(|(f, i)| cx.ix.code[f].nodes[*i].default_name.as_deref())
        .collect();
    let names: Vec<&str> = names.into_iter().collect();
    // Executables built from a shared library link every node class of the package; then
    // take the one whose name the executable is named after (`map_saver` → `map_saver_server`).
    let exec = n.exec.as_deref().unwrap_or_default();
    let named: Vec<&str> = names
        .iter()
        .copied()
        .filter(|d| exec.starts_with(d))
        .collect();
    match (names.as_slice(), named.as_slice()) {
        ([one], _) | (_, [one]) => Some(join_ns(&n.namespace, one)),
        _ => None,
    }
}

fn autostarts(n: &NodeInstance) -> bool {
    n.param("autostart")
        .and_then(|b| b.value.as_ref())
        .and_then(ParamValue::as_bool)
        == Some(true)
}

impl Agent for LifecycleAgent {
    fn id(&self) -> &'static str {
        "lifecycle"
    }
    fn title(&self) -> &'static str {
        "Lifecycle nodes"
    }
    fn inputs(&self) -> &'static [crate::workspace::FileKind] {
        super::GRAPH_INPUTS
    }
    fn relevant(&self, cx: &Context) -> bool {
        cx.graph.instances().any(|(_, _, n)| is_manager(n))
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        for root in &cx.graph.roots {
            check_root(cx, root, out);
        }
    }
}

fn check_root(cx: &Context, root: &RootGraph, out: &mut Vec<Diagnostic>) {
    // Conditional managers (e.g. composition on/off branches) still count for M002: a name
    // must match a node in some branch. M001 needs everything unconditional.
    let managers: Vec<&NodeInstance> = root.nodes.iter().filter(|n| is_manager(n)).collect();
    if managers.is_empty() || root.partial || !root.external_includes.is_empty() {
        return;
    }
    let names: Vec<Option<String>> = root.nodes.iter().map(|n| effective_fq(cx, n)).collect();
    if names.iter().any(Option::is_none) {
        return; // some node's name is unknown: any name in node_names could be it
    }
    let fq = |i: usize| names[i].as_deref().unwrap_or_default();
    // Every manager's list must be known, otherwise we cannot tell what is managed.
    let mut lists = Vec::new();
    for m in &managers {
        let Some(b) = m.param("node_names") else {
            return;
        };
        let Some(ParamValue::StrArray(list)) = &b.value else {
            return;
        };
        lists.push((*m, b, list));
    }
    let launched: BTreeSet<&str> = (0..root.nodes.len()).map(fq).collect();
    let mut managed: BTreeSet<String> = BTreeSet::new();
    for (m, b, list) in &lists {
        for name in list.iter() {
            let target = join_ns(&m.namespace, name);
            managed.insert(target.clone());
            if launched.contains(target.as_str()) {
                continue;
            }
            let leaves: BTreeSet<&str> = (0..root.nodes.len())
                .filter(|i| root.nodes[*i].namespace == m.namespace)
                .filter_map(|i| fq(i).rsplit('/').next())
                .collect();
            let mut d = Diagnostic::new(
                "M002",
                value_span(cx.ix, b.value_loc, name).unwrap_or(b.value_loc),
                format!(
                    "lifecycle manager `{}` manages `{target}`, but no such node is launched; bringup waits for it and fails",
                    m.display_name()
                ),
            );
            if let Some(c) = closest(name, leaves.iter().copied()) {
                d = d.help(format!("Did you mean `{c}`?"));
            }
            if let Some(span) = value_span(cx.ix, b.value_loc, name) {
                d = d.repair(
                    span,
                    name.clone(),
                    leaves.iter().map(|s| s.to_string()).collect(),
                );
            }
            out.push(d);
        }
    }
    if managers.iter().any(|m| m.conditional) {
        return;
    }
    for (i, n) in root.nodes.iter().enumerate() {
        let f = fq(i);
        if n.conditional
            || is_manager(n)
            || managed.contains(f)
            || autostarts(n)
            || !is_lifecycle(cx, n)
        {
            continue;
        }
        let m = lists[0].0;
        out.push(
            Diagnostic::new(
                "M001",
                n.decl.loc,
                format!("lifecycle node `{f}` is launched, but no lifecycle manager here lists it; it stays unconfigured and inactive"),
            )
            .related(m.decl.loc, "lifecycle manager")
            .help(format!("Add `{}` to `node_names` of `{}`.", f.rsplit('/').next().unwrap_or(f), m.display_name())),
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::testutil::analyze_files;

    const PKG: (&str, &str) = (
        "p/package.xml",
        "<package format=\"3\"><name>p</name></package>",
    );
    const CMAKE: (&str, &str) = (
        "p/CMakeLists.txt",
        "project(p)\nadd_executable(planner src/p.cpp)\nadd_executable(mapper src/m.cpp)\ninstall(TARGETS planner mapper DESTINATION lib/${PROJECT_NAME})\n",
    );
    const PLANNER: (&str, &str) = ("p/src/p.cpp", "class P : public rclcpp_lifecycle::LifecycleNode { public: P() : LifecycleNode(\"planner\") {} };\n");
    const MAPPER: (&str, &str) = ("p/src/m.cpp", "class M : public nav2_util::LifecycleNode { public: M() : LifecycleNode(\"mapper\") {} };\n");

    fn launch(names: &str) -> String {
        format!("from launch import LaunchDescription\nfrom launch_ros.actions import Node\ndef generate_launch_description():\n    return LaunchDescription([\n        Node(package='p', executable='planner', name='planner'),\n        Node(package='p', executable='mapper', name='mapper'),\n        Node(package='nav2_lifecycle_manager', executable='lifecycle_manager', name='lifecycle_manager', parameters=[{{'autostart': True, 'node_names': [{names}]}}]),\n    ])\n")
    }

    #[test]
    fn unmanaged_and_unknown_nodes() {
        let l = launch("'planer'");
        let d = analyze_files(&[PKG, CMAKE, PLANNER, MAPPER, ("p/launch/a.launch.py", &l)]);
        let m002 = d.iter().find(|x| x.rule == "M002").expect("M002");
        assert!(
            m002.message.contains("/planer")
                && m002.help.as_deref().is_some_and(|h| h.contains("planner")),
            "{m002:#?}"
        );
        assert_eq!(d.iter().filter(|x| x.rule == "M001").count(), 2, "{d:#?}");
    }

    #[test]
    fn complete_list_is_silent() {
        let l = launch("'planner', 'mapper'");
        let d = analyze_files(&[PKG, CMAKE, PLANNER, MAPPER, ("p/launch/a.launch.py", &l)]);
        assert!(d.iter().all(|x| !x.rule.starts_with('M')), "{d:#?}");
    }
}
