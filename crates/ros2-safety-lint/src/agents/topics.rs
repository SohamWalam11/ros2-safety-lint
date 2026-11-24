//! Topics agent: endpoints that never connect after namespaces and remaps.

use super::{Agent, Context};
use crate::diag::Diagnostic;
use crate::facts::EndpointKind;
use crate::resolve::{instance_endpoints, ImplStatus, ResolvedEndpoint, RootGraph};
use crate::source::Loc;
use crate::util::{edit_distance, max_typo_distance};
use std::collections::{BTreeMap, HashMap, HashSet};

pub struct TopicsAgent;

/// Names commonly provided by infrastructure / created through helpers we do not model.
fn infrastructure(name: &str) -> bool {
    let last = name.rsplit('/').next().unwrap_or(name);
    matches!(
        last,
        "tf" | "tf_static"
            | "clock"
            | "rosout"
            | "parameter_events"
            | "diagnostics"
            | "robot_description"
    ) || name.contains("/_action/")
        || name.ends_with("/transition_event")
}

/// Files that create endpoints through helpers rosfix does not model.
fn uses_unmodelled_helpers(text: &str) -> bool {
    [
        "message_filters",
        "image_transport",
        "TransformListener",
        "TransformBroadcaster",
        "create_generic_",
        "camera_info_manager",
        "point_cloud_transport",
        "laser_geometry",
        "Costmap2DROS",
        "nav2_util::",
        "PluginlibClassLoader",
        "pluginlib",
    ]
    .iter()
    .any(|t| text.contains(t))
}

fn last(n: &str) -> &str {
    n.rsplit('/').next().unwrap_or(n)
}

fn short_type(t: &str) -> String {
    let t = t.trim().trim_start_matches('<').trim_end_matches('>');
    t.rsplit("::")
        .next()
        .unwrap_or(t)
        .rsplit('.')
        .next()
        .unwrap_or(t)
        .trim()
        .to_string()
}

struct RootView<'a> {
    eps: Vec<(usize, ResolvedEndpoint)>,
    closed: bool,
    _root: &'a RootGraph,
}

fn view<'a>(cx: &Context, root: &'a RootGraph) -> RootView<'a> {
    let mut eps = Vec::new();
    let mut closed = !root.partial;
    for (i, n) in root.nodes.iter().enumerate() {
        let (e, complete) = instance_endpoints(cx.ix, n);
        if !complete {
            closed = false;
        }
        eps.extend(e.into_iter().map(|x| (i, x)));
    }
    RootView {
        eps,
        closed,
        _root: root,
    }
}

impl Agent for TopicsAgent {
    fn id(&self) -> &'static str {
        "topics"
    }
    fn title(&self) -> &'static str {
        "Topic wiring (names, namespaces, remaps, types)"
    }
    fn inputs(&self) -> &'static [crate::workspace::FileKind] {
        super::GRAPH_INPUTS
    }
    fn relevant(&self, cx: &Context) -> bool {
        cx.graph
            .instances()
            .any(|(_, _, n)| n.impl_status == ImplStatus::Resolved)
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        for root in &cx.graph.roots {
            let v = view(cx, root);
            near_misses(cx, root, &v, out);
            type_conflicts(&v, out);
            unused_remaps(cx, root, out);
            absolute_in_namespace(cx, root, &v, out);
        }
    }
}

fn counterpart(k: EndpointKind) -> Option<EndpointKind> {
    match k {
        EndpointKind::Subscription => Some(EndpointKind::Publisher),
        EndpointKind::ServiceClient => Some(EndpointKind::ServiceServer),
        EndpointKind::ActionClient => Some(EndpointKind::ActionServer),
        _ => None,
    }
}

fn near_misses(cx: &Context, root: &RootGraph, v: &RootView, out: &mut Vec<Diagnostic>) {
    // BTreeMap: candidates are visited in name order, so the provider named in a T001 message
    // (and therefore the finding's fingerprint) is the same on every run.
    let mut by_kind: HashMap<EndpointKind, BTreeMap<&str, Vec<&ResolvedEndpoint>>> = HashMap::new();
    for (_, e) in &v.eps {
        by_kind
            .entry(e.kind)
            .or_default()
            .entry(e.name.as_str())
            .or_default()
            .push(e);
    }
    let mut reported: HashSet<(Loc, String)> = HashSet::new();
    for (ni, e) in &v.eps {
        let Some(want) = counterpart(e.kind) else {
            continue;
        };
        if infrastructure(&e.name) || cx.config.is_external_topic(&e.name) {
            continue;
        }
        let providers = by_kind.get(&want);
        if providers.is_some_and(|m| m.contains_key(e.name.as_str())) {
            continue;
        }
        // Candidate providers on a similar name.
        let mut best: Option<(&str, &ResolvedEndpoint, bool)> = None;
        if let Some(m) = providers {
            for (pname, peps) in m {
                let same_leaf_other_ns = last(pname) == last(&e.name) && *pname != e.name;
                let typo = edit_distance(pname, &e.name) <= max_typo_distance(last(&e.name)).max(1)
                    && last(pname) != last(&e.name);
                // Outside a closed world only the namespace signal is strong enough.
                if same_leaf_other_ns || (typo && v.closed) {
                    // Don't point at a provider that is itself consumed (it is wired correctly to someone).
                    let consumed = by_kind.get(&e.kind).is_some_and(|c| c.contains_key(pname));
                    if consumed && !same_leaf_other_ns {
                        continue;
                    }
                    if best.is_none() {
                        best = Some((pname, peps[0], same_leaf_other_ns));
                    }
                }
            }
        }
        let node = &root.nodes[*ni];
        match best {
            Some((pname, p, ns_issue)) => {
                if !reported.insert((e.loc, pname.to_string())) {
                    continue;
                }
                let what = if ns_issue {
                    "a different namespace"
                } else {
                    "a similar name"
                };
                let mut d = Diagnostic::new(
                    "T001",
                    e.loc,
                    format!(
                        "{} `{}` of node `{}` has no matching {}, but one exists on {what}: `{pname}`",
                        e.kind.as_str(),
                        e.name,
                        node.display_name(),
                        want.as_str()
                    ),
                )
                .related(p.loc, format!("{} on `{pname}`", want.as_str()))
                .related(node.decl.loc, "node launched here");
                if e.written.starts_with('/') && ns_issue {
                    d = d.help("The name is absolute (starts with `/`), so it ignores the node's namespace. Use a relative name or a remap.");
                } else if ns_issue {
                    d = d.help(
                        "Check the `namespace`/PushRosNamespace of both nodes, or add a remap.",
                    );
                }
                out.push(d);
            }
            None if v.closed && e.kind == EndpointKind::Subscription => {
                out.push(
                    Diagnostic::new(
                        "T005",
                        e.loc,
                        format!(
                            "subscription `{}` of node `{}` has no publisher in this launch graph",
                            e.name,
                            node.display_name()
                        ),
                    )
                    .related(node.decl.loc, "node launched here"),
                );
            }
            None => {}
        }
    }
}

fn type_conflicts(v: &RootView, out: &mut Vec<Diagnostic>) {
    let mut by_topic: HashMap<&str, Vec<&ResolvedEndpoint>> = HashMap::new();
    for (_, e) in &v.eps {
        if e.kind.is_topic() {
            by_topic.entry(e.name.as_str()).or_default().push(e);
        }
    }
    for (topic, eps) in by_topic {
        let typed: Vec<(&ResolvedEndpoint, String)> = eps
            .iter()
            .filter_map(|e| e.msg_type.as_deref().map(|t| (*e, short_type(t))))
            .filter(|(_, t)| !t.is_empty() && t != "?" && !t.contains("SharedPtr"))
            .collect();
        let kinds: HashSet<&str> = typed.iter().map(|(_, t)| t.as_str()).collect();
        if kinds.len() < 2 {
            continue;
        }
        let (first, ft) = &typed[0];
        for (e, t) in &typed[1..] {
            if t != ft {
                out.push(
                    Diagnostic::new(
                        "T003",
                        e.loc,
                        format!(
                            "topic `{topic}` is used with type `{t}` here but `{ft}` elsewhere"
                        ),
                    )
                    .related(first.loc, format!("`{ft}` used here")),
                );
            }
        }
    }
}

fn unused_remaps(cx: &Context, root: &RootGraph, out: &mut Vec<Diagnostic>) {
    for n in &root.nodes {
        if n.impl_status != ImplStatus::Resolved || n.remaps.is_empty() {
            continue;
        }
        let (eps, complete) = instance_endpoints(cx.ix, n);
        if !complete || eps.is_empty() {
            continue;
        }
        if n.impls
            .iter()
            .any(|(f, _)| uses_unmodelled_helpers(&cx.ix.file(*f).text))
        {
            continue;
        }
        let node_name = n.name.clone().unwrap_or_default();
        let expanded: Vec<String> = eps
            .iter()
            .map(|e| crate::resolve::expand_name(&e.written, &n.namespace, &node_name))
            .collect();
        for (from, to, loc) in n.remaps.iter().take(n.decl.remaps.len()) {
            if infrastructure(from)
                || expanded
                    .iter()
                    .any(|x| x == from || x.starts_with(&format!("{from}/")))
            {
                continue;
            }
            let mut d = Diagnostic::new(
                "T002",
                *loc,
                format!(
                    "remap `{from}` → `{to}` has no effect: node `{}` does not use `{from}`",
                    n.display_name()
                ),
            );
            if let Some(c) = crate::util::closest(from, expanded.iter().map(|s| s.as_str())) {
                d = d.help(format!("The node uses `{c}`; did you mean to remap that?"));
            } else {
                let names: Vec<&str> = expanded.iter().map(|s| s.as_str()).take(6).collect();
                d = d.help(format!("Names used by the node: {}", names.join(", ")));
            }
            // Repair site: the remap source as written may become any name the node uses.
            let written_from = n
                .decl
                .remaps
                .iter()
                .find(|(_, _, l)| l == loc)
                .and_then(|(f, _, _)| f.as_lit().map(|s| s.to_string()));
            if let Some(wf) = written_from {
                if let Some(span) = super::value_span(cx.ix, *loc, &wf) {
                    let cands: Vec<String> = eps
                        .iter()
                        .filter(|e| !e.written.starts_with('~'))
                        .map(|e| e.written.clone())
                        .collect();
                    d = d.repair(span, wf, cands);
                }
            }
            out.push(d);
        }
    }
}

fn absolute_in_namespace(cx: &Context, root: &RootGraph, v: &RootView, out: &mut Vec<Diagnostic>) {
    for (ni, e) in &v.eps {
        let n = &root.nodes[*ni];
        if n.namespace == "/"
            || !e.written.starts_with('/')
            || infrastructure(&e.written)
            || cx.config.is_external_topic(&e.written)
            || e.remapped_by.is_some()
        {
            continue;
        }
        out.push(Diagnostic::new(
            "T004",
            e.loc,
            format!("node `{}` runs in namespace `{}` but uses the absolute name `{}`, which ignores the namespace", n.display_name(), n.namespace, e.written),
        ));
    }
}

#[cfg(test)]
mod tests {
    use crate::testutil::analyze_files;

    #[test]
    fn namespace_near_miss_and_unused_remap() {
        let d = analyze_files(&[
            ("p/package.xml", "<package format=\"3\"><name>p</name></package>"),
            ("p/CMakeLists.txt", "project(p)\nadd_executable(talk src/a.cpp)\nadd_executable(listen src/b.cpp)\ninstall(TARGETS talk listen DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch DESTINATION share/${PROJECT_NAME})\n"),
            ("p/src/a.cpp", "class A : public rclcpp::Node { public: A() : Node(\"a\") { pub_ = create_publisher<std_msgs::msg::String>(\"chatter\", 10); } };\n"),
            ("p/src/b.cpp", "class B : public rclcpp::Node { public: B() : Node(\"b\") { sub_ = create_subscription<std_msgs::msg::String>(\"/chatter\", 10, [this](const std_msgs::msg::String::SharedPtr) {}); } };\n"),
            ("p/launch/x.launch.xml", "<launch><node pkg=\"p\" exec=\"talk\" namespace=\"r1\"/><node pkg=\"p\" exec=\"listen\" namespace=\"r1\"><remap from=\"chatter_in\" to=\"x\"/></node></launch>"),
        ]);
        let t001 = d.iter().find(|x| x.rule == "T001").expect("T001");
        assert!(t001.message.contains("/r1/chatter"), "{}", t001.message);
        assert!(d.iter().any(|x| x.rule == "T002"), "{d:#?}");
        assert!(d.iter().any(|x| x.rule == "T004"), "{d:#?}");
    }
}
