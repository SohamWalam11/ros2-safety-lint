//! Launch-argument agent: `DeclareLaunchArgument` / `<arg>` against their uses.
//!
//! - A001 a launch configuration is used (`LaunchConfiguration('x')`, `$(var x)`) without a
//!   default, but no launch file that can be running at that point declares `x` or passes it
//!   to an include. With the default command line, launch fails with "launch configuration
//!   'x' does not exist" (usually a typo such as `use_sim_tme`).
//! - A002 an argument is declared but never mentioned again in its file or anything it
//!   includes (dead option; often the use has a typo).
//!
//! - A003 two unconditional node instances of one launch graph get the same fully qualified
//!   name (only names that are fully known; conditional nodes may be alternatives).
//!
//! Uses are found in the file text, so conditions and `OpaqueFunction` code count as uses.
//! A file that runs in a graph with dynamic launch code (OpaqueFunction, unresolved
//! includes) is not checked; A002 also skips graphs with includes it could not read.

use super::{Agent, Context};
use crate::diag::Diagnostic;
use crate::facts::LaunchItem;
use crate::source::{FileId, Loc};
use crate::util::closest;
use std::collections::{BTreeMap, BTreeSet};

pub struct LaunchArgsAgent;

fn use_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    // Group 1/2: LaunchConfiguration('x' …) with the rest of its argument list; group 3: $(var x).
    RE.get_or_init(|| {
        regex::Regex::new(r#"LaunchConfiguration\(\s*['"]([A-Za-z_][\w]*)['"]([^()]*)\)|\$\(var\s+([A-Za-z_][\w]*)\s*\)"#).expect("valid regex")
    })
}

/// Uses without a default, with their location.
fn uses(cx: &Context, f: FileId) -> Vec<(String, Loc)> {
    let text = &cx.ix.file(f).text;
    use_re()
        .captures_iter(text)
        .filter_map(|c| {
            let m = c.get(0)?;
            if let Some(n) = c.get(1) {
                // `LaunchConfiguration('x', default='…')` never fails.
                if c.get(2)
                    .is_some_and(|rest| rest.as_str().contains("default"))
                {
                    return None;
                }
                return Some((n.as_str().to_string(), Loc::new(f, m.start(), m.end())));
            }
            c.get(3)
                .map(|n| (n.as_str().to_string(), Loc::new(f, m.start(), m.end())))
        })
        .collect()
}

fn collect(items: &[LaunchItem], decls: &mut Vec<(String, Loc)>, passed: &mut BTreeSet<String>) {
    for it in items {
        match it {
            LaunchItem::Arg { name, loc, .. } => decls.push((name.clone(), *loc)),
            LaunchItem::Include { args, .. } => passed.extend(args.iter().map(|(n, _)| n.clone())),
            LaunchItem::Group { items, .. } | LaunchItem::Cond { items, .. } => {
                collect(items, decls, passed)
            }
            _ => {}
        }
    }
}

fn count_includes(items: &[LaunchItem]) -> usize {
    items
        .iter()
        .map(|i| match i {
            LaunchItem::Include { .. } => 1,
            LaunchItem::Group { items, .. } | LaunchItem::Cond { items, .. } => {
                count_includes(items)
            }
            _ => 0,
        })
        .sum()
}

/// Is `name` mentioned anywhere in `text` outside the declaration span?
fn mentioned(text: &str, name: &str, skip: Loc) -> bool {
    let mut from = 0;
    while let Some(i) = text[from..].find(name) {
        let (s, e) = (from + i, from + i + name.len());
        let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        let whole = !word(text[..s].chars().next_back()) && !word(text[e..].chars().next());
        if whole && !(s >= skip.start && e <= skip.end) {
            return true;
        }
        from = e;
    }
    false
}

impl Agent for LaunchArgsAgent {
    fn id(&self) -> &'static str {
        "launch"
    }
    fn title(&self) -> &'static str {
        "Launch arguments"
    }
    fn inputs(&self) -> &'static [crate::workspace::FileKind] {
        super::GRAPH_INPUTS
    }
    fn relevant(&self, cx: &Context) -> bool {
        !cx.ix.launches.is_empty()
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        duplicate_nodes(cx, out);
        // Per file: declarations and names passed to includes.
        let mut decls: BTreeMap<FileId, Vec<(String, Loc)>> = BTreeMap::new();
        let mut passed: BTreeMap<FileId, BTreeSet<String>> = BTreeMap::new();
        for (f, lf) in &cx.ix.launches {
            let (d, p) = (decls.entry(*f).or_default(), passed.entry(*f).or_default());
            collect(&lf.items, d, p);
        }
        // For each file: the argument names available in *every* launch graph that runs it.
        // A name provided by any including graph is fine (it may only ever be included).
        let mut available: BTreeMap<FileId, Option<BTreeSet<String>>> = BTreeMap::new();
        for root in &cx.graph.roots {
            // Declarations can only hide in `OpaqueFunction` code, in declarations the
            // extractor did not understand, or in includes that were not loaded. (Event
            // handlers, lifecycle events or an unresolved namespace mark a graph partial but
            // declare nothing.)
            let hides = |f: &FileId| {
                let text = &cx.ix.file(*f).text;
                let written = text.matches("DeclareLaunchArgument(").count()
                    + text.matches("<arg ").count()
                    + text.matches("<let ").count();
                text.contains("OpaqueFunction") || written > decls.get(f).map_or(0, Vec::len)
            };
            let n_inc: usize = root
                .files
                .iter()
                .filter_map(|f| cx.ix.launches.get(f))
                .map(|l| count_includes(&l.items))
                .sum();
            let dynamic = root.files.iter().any(hides)
                || n_inc > root.files.len().saturating_sub(1)
                || !root.external_includes.is_empty();
            let names: Option<BTreeSet<String>> = (!dynamic).then(|| {
                root.files
                    .iter()
                    .flat_map(|f| {
                        decls
                            .get(f)
                            .into_iter()
                            .flatten()
                            .map(|(n, _)| n.clone())
                            .chain(passed.get(f).into_iter().flatten().cloned())
                    })
                    .collect()
            });
            for f in &root.files {
                let slot = available.entry(*f).or_insert_with(|| Some(BTreeSet::new()));
                match (slot.as_mut(), &names) {
                    (Some(s), Some(n)) => s.extend(n.iter().cloned()),
                    _ => *slot = None, // reached from a partial graph: unknown
                }
            }
        }
        // An argument may be used by an included file we could not read: skip graphs with
        // dynamic content, external includes, or includes that were not loaded.
        let complete = |r: &crate::resolve::RootGraph| {
            let n_inc: usize = r
                .files
                .iter()
                .filter_map(|f| cx.ix.launches.get(f))
                .map(|l| count_includes(&l.items))
                .sum();
            r.external_includes.is_empty()
                && r.files
                    .iter()
                    .all(|f| cx.ix.launches.get(f).is_some_and(|l| !l.partial))
                && n_inc <= r.files.len().saturating_sub(1)
        };
        // Files another launch file may include through an include we could not load: a
        // graph that is not complete and whose root mentions the file's name.
        let maybe_included: BTreeSet<FileId> =
            cx.ix
                .launches
                .keys()
                .copied()
                .filter(|f| {
                    let rel = &cx.ix.file(*f).rel_path;
                    let base = rel.rsplit('/').next().unwrap_or(rel);
                    cx.graph.roots.iter().any(|r| {
                        r.root != *f && !complete(r) && cx.ix.file(r.root).text.contains(base)
                    })
                })
                .collect();
        for (f, avail) in &available {
            let Some(avail) = avail else { continue };
            if cx.ix.external_files.contains(f) || maybe_included.contains(f) {
                continue;
            }
            let mut seen = BTreeSet::new();
            for (name, loc) in uses(cx, *f) {
                if avail.contains(&name) || !seen.insert(name.clone()) {
                    continue;
                }
                let mut d = Diagnostic::new(
                    "A001",
                    loc,
                    format!("launch configuration `{name}` is used but never declared here or by any launch file that includes this one; launch fails unless `{name}:=…` is given on the command line"),
                );
                if let Some(c) = closest(&name, avail.iter().map(String::as_str)) {
                    d = d.help(format!("Did you mean `{c}`?"));
                } else {
                    d = d.help(format!("Declare it with `DeclareLaunchArgument('{name}', default_value=…)` / `<arg name=\"{name}\" default=\"…\"/>`."));
                }
                out.push(d);
            }
        }
        // A002: declared, never mentioned again in the file or anything it includes.
        for root in cx.graph.roots.iter().filter(|r| complete(r)) {
            let f = root.root;
            if cx.ix.external_files.contains(&f) {
                continue;
            }
            for (name, loc) in decls.get(&f).into_iter().flatten() {
                let used = root.files.iter().any(|g| {
                    mentioned(
                        &cx.ix.file(*g).text,
                        name,
                        if *g == f { *loc } else { Loc::new(*g, 0, 0) },
                    )
                });
                if !used {
                    out.push(Diagnostic::new("A002", *loc, format!("launch argument `{name}` is declared but never used in this launch file or the files it includes")));
                }
            }
        }
    }
}

/// A003: the first instance of each fully qualified name, then every later one with the same
/// name. A name counts only when it is certain: given in the launch file, or taken from
/// workspace source (installed executables may add a random suffix to their default name).
fn duplicate_nodes(cx: &Context, out: &mut Vec<Diagnostic>) {
    use crate::resolve::ImplStatus;
    for root in &cx.graph.roots {
        let mut first: BTreeMap<&str, Loc> = BTreeMap::new();
        for n in root.nodes.iter().filter(|n| !n.conditional) {
            let Some(fq) = n.fq_name.as_deref() else {
                continue;
            };
            if n.decl.name.is_none() && n.impl_status != ImplStatus::Resolved {
                continue;
            }
            match first.get(fq) {
                None => {
                    first.insert(fq, n.decl.loc);
                }
                Some(&prev) => out.push(
                    Diagnostic::new("A003", n.decl.loc, format!("a second node is launched as `{fq}`; the two share one name, its parameter services and its parameter-file sections"))
                        .related(prev, format!("first node named `{fq}`"))
                        .help("Give one of them a different `name` or `namespace`."),
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::testutil::analyze_files;

    const PKG: (&str, &str) = (
        "p/package.xml",
        "<package format=\"3\"><name>p</name></package>",
    );

    #[test]
    fn undeclared_and_unused_arguments() {
        let py = "from launch import LaunchDescription\nfrom launch.actions import DeclareLaunchArgument\nfrom launch.substitutions import LaunchConfiguration\nfrom launch_ros.actions import Node\ndef generate_launch_description():\n    return LaunchDescription([\n        DeclareLaunchArgument('use_sim_time', default_value='false'),\n        DeclareLaunchArgument('robot_name', default_value='r1'),\n        Node(package='p', executable='x', parameters=[{'use_sim_time': LaunchConfiguration('use_sim_tme')}]),\n        Node(package='p', executable='y', namespace=LaunchConfiguration('ns', default='')),\n    ])\n";
        let d = analyze_files(&[PKG, ("p/launch/a.launch.py", py)]);
        let a001: Vec<_> = d.iter().filter(|x| x.rule == "A001").collect();
        assert_eq!(a001.len(), 1, "{a001:#?}");
        assert!(
            a001[0].message.contains("use_sim_tme")
                && a001[0]
                    .help
                    .as_deref()
                    .is_some_and(|h| h.contains("use_sim_time")),
            "{a001:#?}"
        );
        let a002: Vec<&str> = d
            .iter()
            .filter(|x| x.rule == "A002")
            .map(|x| x.message.as_str())
            .collect();
        // `use_sim_time` is mentioned again (as a parameter key): any mention counts as a use.
        assert_eq!(a002.len(), 1, "{a002:#?}");
        assert!(a002[0].contains("`robot_name`"), "{a002:#?}");
    }

    #[test]
    fn arguments_from_including_file_and_conditions_are_fine() {
        let child = "<launch>\n<node pkg=\"p\" exec=\"x\" namespace=\"$(var ns)\"/>\n</launch>\n";
        let parent = "<launch>\n<arg name=\"ns\" default=\"r1\"/>\n<arg name=\"gui\" default=\"true\"/>\n<include file=\"$(find-pkg-share p)/launch/child.launch.xml\"/>\n<node pkg=\"p\" exec=\"rviz\" if=\"$(var gui)\"/>\n</launch>\n";
        let d = analyze_files(&[
            PKG,
            ("p/launch/child.launch.xml", child),
            ("p/launch/parent.launch.xml", parent),
        ]);
        assert!(
            d.iter().all(|x| x.rule != "A001" && x.rule != "A002"),
            "{d:#?}"
        );
    }

    #[test]
    fn duplicate_node_names() {
        let xml = "<launch>\n<node pkg=\"p\" exec=\"cam\" name=\"camera\"/>\n<node pkg=\"p\" exec=\"cam2\" name=\"camera\"/>\n<node pkg=\"p\" exec=\"cam\" name=\"camera\" namespace=\"left\"/>\n<node pkg=\"p\" exec=\"cam\" name=\"rviz\" if=\"$(var gui)\"/>\n<node pkg=\"p\" exec=\"cam\" name=\"rviz\" unless=\"$(var gui)\"/>\n<node pkg=\"tf2_ros\" exec=\"static_transform_publisher\" args=\"--frame-id a --child-frame-id b\"/>\n<node pkg=\"tf2_ros\" exec=\"static_transform_publisher\" args=\"--frame-id a --child-frame-id c\"/>\n<arg name=\"gui\" default=\"true\"/>\n</launch>\n";
        let d = analyze_files(&[PKG, ("p/launch/a.launch.xml", xml)]);
        let a003: Vec<&str> = d
            .iter()
            .filter(|x| x.rule == "A003")
            .map(|x| x.message.as_str())
            .collect();
        // Only `/camera` twice: namespaced, conditional alternatives and nameless installed
        // executables (which may pick a unique default name) are not reported.
        assert_eq!(a003.len(), 1, "{a003:#?}");
        assert!(a003[0].contains("`/camera`"), "{a003:#?}");
    }

    /// rosbridge_websocket_launch.xml and image_proc.launch.py: the same node on two exclusive
    /// branches (`<group if>`/`<group unless>`; a conditional container vs LoadComposableNodes).
    #[test]
    fn nodes_on_exclusive_branches_are_not_duplicates() {
        let xml = "<launch>\n<arg name=\"ssl\" default=\"false\"/>\n<group if=\"$(var ssl)\"><node pkg=\"p\" exec=\"ws\" name=\"bridge\"/></group>\n<group unless=\"$(var ssl)\"><node pkg=\"p\" exec=\"ws\" name=\"bridge\"/></group>\n</launch>";
        let py = "from launch import LaunchDescription\nfrom launch.conditions import LaunchConfigurationEquals, LaunchConfigurationNotEquals\nfrom launch_ros.actions import ComposableNodeContainer, LoadComposableNodes\nfrom launch_ros.descriptions import ComposableNode\ndef generate_launch_description():\n    nodes = [ComposableNode(package='p', plugin='p::Debayer', name='debayer_node')]\n    return LaunchDescription([\n        ComposableNodeContainer(name='c', namespace='', package='rclcpp_components', executable='component_container', composable_node_descriptions=nodes, condition=LaunchConfigurationEquals('container', '')),\n        LoadComposableNodes(composable_node_descriptions=nodes, target_container='other', condition=LaunchConfigurationNotEquals('container', '')),\n    ])\n";
        let d = analyze_files(&[
            PKG,
            ("p/launch/a.launch.xml", xml),
            ("p/launch/b.launch.py", py),
        ]);
        assert!(d.iter().all(|x| x.rule != "A003"), "{d:#?}");
    }

    /// turtlebot4_gz: a node under a namespace that cannot be evaluated still gets the
    /// parameter-file sections that match in any namespace (`/**`), and only those.
    #[test]
    fn unknown_namespace_binds_namespace_independent_keys() {
        let cmake = ("p/CMakeLists.txt", "project(p)\nadd_executable(n src/n.cpp)\ninstall(TARGETS n DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch config DESTINATION share/${PROJECT_NAME})\n");
        let cpp = ("p/src/n.cpp", "class N : public rclcpp::Node { public: N() : Node(\"node\") { declare_parameter(\"rate\", 1.0); declare_parameter(\"gain\", 1.0); } };\n");
        let launch = "<launch><group><push-ros-namespace namespace=\"$(eval 'r' + '1')\"/><node pkg=\"p\" exec=\"n\" name=\"node\"><param from=\"$(find-pkg-share p)/config/a.yaml\"/></node></group></launch>";
        let yaml =
            "/**:\n  ros__parameters:\n    rate: 2\nnode:\n  ros__parameters:\n    gain: 3\n";
        let d = analyze_files(&[
            PKG,
            cmake,
            cpp,
            ("p/config/a.yaml", yaml),
            ("p/launch/a.launch.xml", launch),
        ]);
        let p004: Vec<&str> = d
            .iter()
            .filter(|x| x.rule == "P004")
            .map(|x| x.message.as_str())
            .collect();
        // `/**` applies whatever the namespace; `node:` means `/node`, which depends on it.
        assert_eq!(p004.len(), 1, "{p004:#?}");
        assert!(p004[0].contains("`rate`"), "{p004:#?}");
        assert!(d.iter().all(|x| x.rule != "P001"), "{d:#?}");
    }

    /// mavros multi_uas.launch: the same file included twice under namespaces computed by
    /// `$(eval …)`. The names differ at runtime; statically they are unknown, not equal.
    #[test]
    fn unresolved_namespaces_are_not_duplicates() {
        let child = "<launch><arg name=\"namespace\" default=\"\"/><node pkg=\"p\" exec=\"cam\" name=\"camera\" namespace=\"$(var namespace)\"/></launch>";
        let parent = "<launch>\n<group><include file=\"$(find-pkg-share p)/launch/child.launch.xml\"><arg name=\"namespace\" value=\"$(eval 'uas_' + '1')\"/></include></group>\n<group><include file=\"$(find-pkg-share p)/launch/child.launch.xml\"><arg name=\"namespace\" value=\"$(eval 'uas_' + '2')\"/></include></group>\n<group><push-ros-namespace namespace=\"$(eval 'x')\"/><node pkg=\"p\" exec=\"cam\" name=\"other\"/></group>\n<group><push-ros-namespace namespace=\"$(eval 'y')\"/><node pkg=\"p\" exec=\"cam\" name=\"other\"/></group>\n</launch>";
        let d = analyze_files(&[
            PKG,
            ("p/launch/child.launch.xml", child),
            ("p/launch/parent.launch.xml", parent),
        ]);
        assert!(d.iter().all(|x| x.rule != "A003"), "{d:#?}");
    }
}
