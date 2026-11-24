//! Parameters agent: values that never reach a node, or crash it at startup.

use super::{Agent, Context};
use crate::diag::{Diagnostic, Fix, FixSafety, TextEdit};
use crate::facts::*;
use crate::resolve::{declared_params, is_load_marker, ImplStatus, NodeInstance};
use crate::util::closest;
use std::collections::HashSet;

pub struct ParamsAgent;

/// Parameters every node accepts without declaring them in its own code.
const IMPLICIT: &[&str] = &[
    "use_sim_time",
    "start_type_description_service",
    "start_parameter_event_publisher",
];

fn implicit(name: &str) -> bool {
    IMPLICIT.contains(&name) || name.starts_with("qos_overrides.")
}

impl Agent for ParamsAgent {
    fn id(&self) -> &'static str {
        "params"
    }
    fn title(&self) -> &'static str {
        "Parameters (silently ignored values, type errors)"
    }
    fn inputs(&self) -> &'static [crate::workspace::FileKind] {
        super::GRAPH_INPUTS
    }
    fn relevant(&self, cx: &Context) -> bool {
        !cx.ix.param_files.is_empty() || cx.graph.instances().any(|(_, _, n)| !n.params.is_empty())
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        p002_missing_ros_parameters(cx, out);
        p001_unmatched_keys(cx, out);
        super::cross::run(cx, out);
        for (_, _, inst) in cx.graph.instances() {
            p003_p004(cx, inst, out);
            p005_required(cx, inst, out);
        }
    }
}

fn p002_missing_ros_parameters(cx: &Context, out: &mut Vec<Diagnostic>) {
    for pf in cx.ix.param_files.values() {
        for m in &pf.missing_ros_parameters {
            match &m.misspelled {
                Some((word, loc)) => out.push(
                    Diagnostic::new("P002", *loc, format!("`{word}` should be `ros__parameters` (two underscores); the values under node `{}` are ignored", m.node_key))
                        .safe_fix("Rename the key to `ros__parameters`", loc.file, loc.start, loc.end, "ros__parameters"),
                ),
                None => out.push(
                    Diagnostic::new("P002", m.key_loc, format!("node section `{}` has no `ros__parameters:` level, so none of its values are applied", m.node_key))
                        .help("Nest the values under `ros__parameters:` (indented one level below the node name)."),
                ),
            }
        }
    }
}

/// Controller names configured through controller_manager are node keys too.
fn controller_names(pf: &ParamFile) -> HashSet<String> {
    let mut names = HashSet::new();
    for s in &pf.sections {
        let last = s.node_key.rsplit('/').next().unwrap_or("");
        if last == "controller_manager" {
            for e in &s.params {
                if let Some(c) = e.name.strip_suffix(".type") {
                    names.insert(c.to_string());
                }
            }
        }
        if s.params
            .iter()
            .any(|e| e.name == "type" && matches!(&e.value, ParamValue::Str(v) if v.contains('/')))
        {
            names.insert(last.to_string());
        }
    }
    names
}

fn p001_unmatched_keys(cx: &Context, out: &mut Vec<Diagnostic>) {
    for (fid, loads) in &cx.graph.file_loads {
        let Some(pf) = cx.ix.param_files.get(fid) else {
            continue;
        };
        let insts: Vec<&NodeInstance> = loads
            .iter()
            .map(|(r, n)| &cx.graph.roots[*r].nodes[*n])
            .collect();
        // Need every loader's name to be known to claim "matches nothing".
        if insts
            .iter()
            .any(|n| n.fq_name.is_none() || !n.param_sources_complete)
        {
            continue;
        }
        // Another node in the same launch graph might load this file through a source we
        // could not resolve; then "matches no loader" cannot be claimed.
        let roots: HashSet<usize> = loads.iter().map(|(r, _)| *r).collect();
        if roots.iter().any(|r| {
            cx.graph.roots[*r]
                .nodes
                .iter()
                .any(|n| !n.param_sources_complete)
        }) {
            continue;
        }
        let controllers = controller_names(pf);
        for s in &pf.sections {
            let last = s.node_key.rsplit('/').next().unwrap_or("");
            if controllers.contains(last) || s.params.is_empty() {
                continue;
            }
            let matched = insts.iter().any(|n| {
                n.params.iter().any(|b| {
                    b.from_file == Some(*fid)
                        && !is_load_marker(b)
                        && s.params.iter().any(|e| e.key_loc == b.key_loc)
                })
            });
            if matched {
                continue;
            }
            let fqs: Vec<String> = insts
                .iter()
                .filter_map(|n| n.fq_name.clone())
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            // Same node name in another namespace: the classic namespacing bug.
            let same_name: Vec<&String> = fqs
                .iter()
                .filter(|f| f.rsplit('/').next() == Some(last))
                .collect();
            let mut d = if let Some(fq) = same_name.first() {
                Diagnostic::new(
                    "P001",
                    s.key_loc,
                    format!("parameter key `{}` matches no node: `{last}` is launched as `{fq}`, so these {} value(s) are silently ignored", s.node_key, s.params.len()),
                )
                .help(format!("Use the fully-qualified key `{fq}` or a wildcard such as `/**/{last}`."))
                .fix(Fix {
                    description: format!("Use `/**/{last}` so the key matches in any namespace"),
                    safety: FixSafety::Suggestion,
                    edits: vec![TextEdit { file: s.key_loc.file, start: s.key_loc.start, end: s.key_loc.end, replacement: format!("/**/{last}") }],
                })
            } else {
                // Without a clear relation to a loader the key may belong to a node created
                // inside another node (e.g. Nav2 costmaps) — only report likely typos.
                let names: Vec<&str> = fqs
                    .iter()
                    .map(|f| f.rsplit('/').next().unwrap_or(""))
                    .collect();
                let Some(c) = closest(last, names.iter().copied()) else {
                    continue;
                };
                Diagnostic::new(
                    "P001",
                    s.key_loc,
                    format!("parameter key `{}` matches none of the nodes that load this file ({}); its {} value(s) are silently ignored", s.node_key, fqs.join(", "), s.params.len()),
                )
                .help(format!("Did you mean `{c}`?"))
                .safe_fix(format!("Rename the key to `{c}`"), s.key_loc.file, s.key_loc.start, s.key_loc.end, c)
            };
            for n in insts.iter().take(3) {
                d = d.related(n.decl.loc, format!("loaded by `{}`", n.display_name()));
            }
            out.push(d);
        }
    }
}

fn type_compatible(declared: ParamType, given: ParamType) -> bool {
    declared == ParamType::Unknown || given == ParamType::Unknown || declared == given
}

fn p003_p004(cx: &Context, inst: &NodeInstance, out: &mut Vec<Diagnostic>) {
    if inst.impl_status != ImplStatus::Resolved {
        return;
    }
    let decls = declared_params(cx.ix, inst);
    let impl_desc = impl_description(cx, inst);
    for b in &inst.params {
        if is_load_marker(b) || implicit(&b.name) {
            continue;
        }
        let decl = decls.iter().find(|d| d.name.matches(&b.name));
        // An unevaluated value (e.g. `{"ns": ParameterBuilder(..).to_dict()}`) may be a dict
        // that expands to `ns.*`: if the node declares parameters under `ns.`, it is not undeclared.
        // Only Python launch values can be such dicts: XML/YAML `<param value=…>` is a scalar.
        let from_python =
            cx.ix.kinds.get(b.key_loc.file) == Some(&crate::workspace::FileKind::LaunchPy);
        let opaque = from_python && matches!(b.value, None | Some(ParamValue::Unknown));
        let prefix = format!("{}.", b.name);
        // Dotted names are open (plugins, a listener with an unknown prefix): the dict may
        // expand to any of them.
        if decl.is_none()
            && opaque
            && (inst.params_open_dotted
                || decls.iter().any(|d| d.name.literal().starts_with(&prefix)))
        {
            continue;
        }
        let can_check_missing =
            !inst.params_open && !(inst.params_open_dotted && b.name.contains('.'));
        match decl {
            None if can_check_missing => {
                if let Some(d) = renamed_between_distros(cx, inst, &b.name, b.key_loc, &decls) {
                    out.push(d.related(inst.decl.loc, "node launched here"));
                    continue;
                }
                let exact: Vec<&str> = decls
                    .iter()
                    .filter_map(|d| match &d.name {
                        NamePattern::Exact(n) => Some(n.as_str()),
                        _ => None,
                    })
                    .collect();
                let mut d = Diagnostic::new(
                    "P003",
                    b.key_loc,
                    format!("`{}` is not declared by node `{}` ({impl_desc}), so this value is silently ignored", b.name, inst.display_name()),
                )
                .related(inst.decl.loc, "node launched here");
                // Repair site for the fix assistant: the key may become any declared sibling
                // name that is not already set for this node.
                let key_text = cx.ix.file(b.key_loc.file).slice(b.key_loc);
                let tail = b.name.rsplit('.').next().unwrap_or(&b.name);
                if b.from_file.is_some() && key_text == tail {
                    let prefix = b.name.rsplit_once('.').map(|x| x.0);
                    let already: std::collections::HashSet<&str> =
                        inst.params.iter().map(|p| p.name.as_str()).collect();
                    let cands: Vec<String> = exact
                        .iter()
                        .filter(|n| {
                            n.rsplit_once('.').map(|x| x.0) == prefix && !already.contains(**n)
                        })
                        .map(|n| n.rsplit('.').next().unwrap_or(n).to_string())
                        .collect();
                    d = d.repair(b.key_loc, key_text, cands);
                }
                if let Some(c) = closest(&b.name, exact.iter().copied()) {
                    d = d.help(format!("Did you mean `{c}`?"));
                    if b.from_file.is_some() {
                        // Only the last segment is written as the key in nested YAML.
                        let (old_tail, new_tail) = (
                            b.name.rsplit('.').next().unwrap_or(&b.name),
                            c.rsplit('.').next().unwrap_or(c),
                        );
                        let same_prefix =
                            b.name.rsplit_once('.').map(|x| x.0) == c.rsplit_once('.').map(|x| x.0);
                        let key_text = cx.ix.file(b.key_loc.file).slice(b.key_loc);
                        if same_prefix && key_text == old_tail {
                            d = d.safe_fix(
                                format!("Rename `{old_tail}` to `{new_tail}`"),
                                b.key_loc.file,
                                b.key_loc.start,
                                b.key_loc.end,
                                new_tail,
                            );
                        }
                    }
                }
                out.push(d);
            }
            Some(decl) => {
                let Some(v) = &b.value else { continue };
                let given = v.ty();
                if type_compatible(decl.ty, given) {
                    if let Some(why) = decl.constraints.iter().find_map(|c| c.violated_by(v)) {
                        out.push(
                            Diagnostic::new(
                                "P006",
                                b.value_loc,
                                format!("`{}` = {} violates {why} declared by node `{}`; the node throws at startup", b.name, v.show(), inst.display_name()),
                            )
                            .related(decl.loc, "constraint declared here"),
                        );
                    }
                    continue;
                }
                let mut d = Diagnostic::new(
                    "P004",
                    b.value_loc,
                    format!(
                        "`{}` is declared as {} by node `{}` but set to {} here; the node will throw at startup",
                        b.name,
                        decl.ty.as_str(),
                        inst.display_name(),
                        given.as_str()
                    ),
                )
                .related(decl.loc, format!("declared as {} here", decl.ty.as_str()));
                if b.from_file.is_some() {
                    match (decl.ty, given, b.raw.as_deref()) {
                        (ParamType::Double, ParamType::Int, Some(raw)) => {
                            d = d.safe_fix(
                                format!("Write `{raw}.0`"),
                                b.value_loc.file,
                                b.value_loc.start,
                                b.value_loc.end,
                                format!("{raw}.0"),
                            );
                        }
                        (ParamType::Str, _, Some(raw)) if !raw.contains('"') => {
                            d = d.safe_fix(
                                format!("Quote the value: `\"{raw}\"`"),
                                b.value_loc.file,
                                b.value_loc.start,
                                b.value_loc.end,
                                format!("\"{raw}\""),
                            );
                        }
                        _ => {}
                    }
                }
                out.push(d);
            }
            None => {}
        }
    }
}

fn p005_required(cx: &Context, inst: &NodeInstance, out: &mut Vec<Diagnostic>) {
    if inst.impl_status != ImplStatus::Resolved || !inst.param_sources_complete || inst.conditional
    {
        return;
    }
    for (f, i) in &inst.impls {
        let imp = &cx.ix.code[f].nodes[*i];
        if imp.lang != Lang::Cpp {
            continue;
        }
        for d in &imp.params {
            if d.has_default || d.ty == ParamType::Unknown {
                continue;
            }
            let NamePattern::Exact(name) = &d.name else {
                continue;
            };
            if inst.params.iter().any(|b| b.name == *name) {
                continue;
            }
            out.push(
                Diagnostic::new(
                    "P005",
                    d.loc,
                    format!("`{name}` has no default and node `{}` is launched without a value for it; declaring it will throw", inst.display_name()),
                )
                .related(inst.decl.loc, "launched here without the parameter"),
            );
        }
    }
}

fn impl_description(cx: &Context, inst: &NodeInstance) -> String {
    let mut parts: Vec<String> = inst
        .impls
        .iter()
        .map(|(f, i)| {
            let n = &cx.ix.code[f].nodes[*i];
            let file = &cx.ix.file(*f).rel_path;
            match &n.class_name {
                Some(c) => format!("class `{c}` in {file}"),
                None => file.clone(),
            }
        })
        .collect();
    parts.dedup();
    parts.truncate(2);
    parts.join(", ")
}

/// The other distributions' knowledge packs, decoded once.
fn other_packs() -> &'static [(&'static str, Option<crate::pack::Pack>)] {
    static PACKS: std::sync::OnceLock<Vec<(&'static str, Option<crate::pack::Pack>)>> =
        std::sync::OnceLock::new();
    PACKS.get_or_init(|| {
        ["humble", "jazzy", "kilted"]
            .into_iter()
            .map(|d| (d, crate::pack::embedded(d)))
            .collect()
    })
}

/// P007: a parameter that the node (from an installed package, known through the knowledge
/// pack) does not declare in this distribution but does in another one: it was renamed or
/// removed between releases.
fn renamed_between_distros(
    cx: &Context,
    inst: &NodeInstance,
    name: &str,
    at: crate::source::Loc,
    decls: &[&ParamDecl],
) -> Option<Diagnostic> {
    let [(f, i)] = inst.impls.as_slice() else {
        return None;
    };
    let rel = &cx.ix.file(*f).rel_path;
    let rest = rel.strip_prefix("<rosfix knowledge pack: ")?;
    let (distro, pkg) = rest.split_once(">/")?;
    let key = cx.ix.code.get(f)?.nodes.get(*i)?.class_name.clone()?;
    let current = if distro == "rolling" {
        "kilted"
    } else {
        distro
    };
    let elsewhere: Vec<&str> = other_packs()
        .iter()
        .filter(|(d, _)| *d != current)
        .filter(|(_, p)| {
            p.as_ref()
                .and_then(|p| p.packages.get(pkg))
                .and_then(|pp| pp.executables.get(&key).or_else(|| pp.components.get(&key)))
                .is_some_and(|n| n.params.iter().any(|x| x.name.matches(name)))
        })
        .map(|(d, _)| *d)
        .collect();
    if elsewhere.is_empty() {
        return None;
    }
    let cap = |d: &str| {
        let mut c = d.chars();
        c.next()
            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
            .unwrap_or_default()
    };
    let list = elsewhere
        .iter()
        .map(|d| cap(d))
        .collect::<Vec<_>>()
        .join(" and ");
    let mut d = Diagnostic::new(
        "P007",
        at,
        // No distribution names in the message: findings are fingerprinted by message (baselines,
        // `rosfix migrate`), and the same finding must not change with ROS_DISTRO.
        format!("`{name}` is not a parameter of `{pkg}/{key}` in this ROS distribution but is in another one; it was renamed or removed, so this value is silently ignored"),
    )
    .help(format!("Declared in {list}, not in {} (the distribution analysed).", cap(distro)));
    let exact: Vec<&str> = decls
        .iter()
        .filter_map(|d| match &d.name {
            NamePattern::Exact(n) => Some(n.as_str()),
            _ => None,
        })
        .collect();
    if let Some(c) = closest(name, exact.iter().copied()) {
        d = d.help(format!("Declared in {list}, not in {} (the distribution analysed); there the closest parameter is `{c}`.", cap(distro)));
    }
    Some(d)
}

#[cfg(test)]
mod tests {
    use crate::testutil::analyze_files;

    /// `{"ns": ParameterBuilder(..).to_dict()}` (moveit_servo demos) sets `ns.*`, which the
    /// node declares; the key alone must not be reported as undeclared.
    #[test]
    fn opaque_value_under_declared_namespace() {
        let d = analyze_files(&[
            ("demo/package.xml", "<package format=\"3\"><name>demo</name></package>"),
            ("demo/CMakeLists.txt", "project(demo)\nadd_executable(n src/n.cpp)\ninstall(TARGETS n DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch DESTINATION share/${PROJECT_NAME})\n"),
            ("demo/src/n.cpp", "class N : public rclcpp::Node { public: N() : Node(\"n\") { declare_parameter(\"servo.rate\", 1.0); } };\n"),
            ("demo/launch/a.launch.py", "from launch import LaunchDescription\nfrom launch_ros.actions import Node\nfrom launch_param_builder import ParameterBuilder\ndef generate_launch_description():\n    p = {'servo': ParameterBuilder('demo').yaml('c.yaml').to_dict()}\n    return LaunchDescription([Node(package='demo', executable='n', parameters=[p, {'other': 1}])])\n"),
        ]);
        let p003: Vec<_> = d.iter().filter(|x| x.rule == "P003").collect();
        assert_eq!(p003.len(), 1, "{p003:#?}");
        assert!(p003[0].message.contains("`other`"), "{}", p003[0].message);
    }

    /// moveit_servo demos: `ParamListener(node, param_namespace)` with
    /// `const std::string param_namespace = "moveit_servo";` declares the generate_parameter_library
    /// parameters as `moveit_servo.*`, and the launch file passes `{"moveit_servo": ...to_dict()}`.
    #[test]
    fn param_listener_prefix_declares_namespaced_library_params() {
        let files = |prefix_src: &str| {
            let cpp = format!("#include <rclcpp/rclcpp.hpp>\nint main() {{\n  auto node = std::make_shared<rclcpp::Node>(\"servo_demo\");\n  {prefix_src}\n  auto l = std::make_shared<const servo::ParamListener>(node, param_namespace);\n}}\n");
            vec![
                ("s/package.xml".to_string(), "<package format=\"3\"><name>s</name></package>".to_string()),
                ("s/CMakeLists.txt".into(), "project(s)\ngenerate_parameter_library(servo_params config/servo.yaml)\nadd_executable(demo src/demo.cpp)\ninstall(TARGETS demo DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch DESTINATION share/${PROJECT_NAME})\n".into()),
                ("s/config/servo.yaml".into(), "servo:\n  move_group_name:\n    type: string\n    default_value: arm\n".into()),
                ("s/src/demo.cpp".into(), cpp),
                ("s/launch/a.launch.py".into(), "from launch import LaunchDescription\nfrom launch_ros.actions import Node\nfrom launch_param_builder import ParameterBuilder\ndef generate_launch_description():\n    p = {'moveit_servo': ParameterBuilder('s').yaml('c.yaml').to_dict()}\n    return LaunchDescription([Node(package='s', executable='demo', parameters=[p, {'other': 1}])])\n".into()),
            ]
        };
        for prefix_src in [
            "const std::string param_namespace = \"moveit_servo\";",
            "std::string param_namespace = get_ns();",
        ] {
            let f = files(prefix_src);
            let refs: Vec<(&str, &str)> = f.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
            let d = analyze_files(&refs);
            let p003: Vec<&str> = d
                .iter()
                .filter(|x| x.rule == "P003")
                .map(|x| x.message.as_str())
                .collect();
            assert!(
                p003.iter().all(|m| !m.contains("`moveit_servo`")),
                "{prefix_src}: {p003:#?}"
            );
            assert!(
                p003.iter().any(|m| m.contains("`other`")),
                "{prefix_src}: {p003:#?}"
            );
        }
    }

    /// moveit2 Servo demos: `update_period` and `planning_group_name` are declared by
    /// moveit_core's acceleration filter (`online_signal_smoothing::ParamListener(node_)`), a
    /// plugin that moveit_servo loads into the demo node. The dependency's spec declares them.
    #[test]
    fn dependency_param_library_declares_on_hosting_node() {
        let d = analyze_files(&[
            ("core/package.xml", "<package format=\"3\"><name>core</name></package>"),
            // As in moveit_core, the parameter library is generated in a subdirectory.
            ("core/CMakeLists.txt", "project(core)\nadd_subdirectory(smoothing)\n"),
            ("core/smoothing/CMakeLists.txt", "generate_parameter_library(accel_params src/accel.yaml)\nadd_library(accel src/accel.cpp)\n"),
            ("core/smoothing/src/accel.yaml", "online_signal_smoothing:\n  update_period:\n    type: double\n  planning_group_name:\n    type: string\n"),
            ("core/smoothing/src/accel.cpp", "#include <rclcpp/rclcpp.hpp>\nvoid AccelerationFilter::initialize(rclcpp::Node::SharedPtr node) {\n  auto l = online_signal_smoothing::ParamListener(node);\n}\n"),
            ("servo/package.xml", "<package format=\"3\"><name>servo</name><depend>core</depend></package>"),
            ("servo/CMakeLists.txt", "project(servo)\nadd_executable(demo src/demo.cpp)\ninstall(TARGETS demo DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch DESTINATION share/${PROJECT_NAME})\n"),
            ("servo/src/demo.cpp", "#include <rclcpp/rclcpp.hpp>\nint main() {\n  auto node = std::make_shared<rclcpp::Node>(\"servo_demo\");\n  node->declare_parameter(\"rate\", 1.0);\n}\n"),
            ("servo/launch/a.launch.py", "from launch import LaunchDescription\nfrom launch_ros.actions import Node\ndef generate_launch_description():\n    return LaunchDescription([Node(package='servo', executable='demo', parameters=[{'update_period': 0.01}, {'planning_group_name': 'arm'}, {'other': 1}])])\n"),
        ]);
        let p003: Vec<&str> = d
            .iter()
            .filter(|x| x.rule == "P003")
            .map(|x| x.message.as_str())
            .collect();
        assert!(
            p003.iter()
                .all(|m| !m.contains("`update_period`") && !m.contains("`planning_group_name`")),
            "{p003:#?}"
        );
        assert!(p003.iter().any(|m| m.contains("`other`")), "{p003:#?}");
    }

    /// rmf_ros2: an XML `<param value="… $(var x) …">` is a scalar even when it cannot be
    /// evaluated, so an undeclared name is reported even if the node's dotted names are open.
    #[test]
    fn unevaluated_xml_param_is_still_checked() {
        let d = analyze_files(&[
            ("s/package.xml", "<package format=\"3\"><name>s</name></package>"),
            ("s/CMakeLists.txt", "project(s)\nadd_executable(agg src/a.cpp)\ninstall(TARGETS agg DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch DESTINATION share/${PROJECT_NAME})\n"),
            ("s/src/a.cpp", "#include <rclcpp/rclcpp.hpp>\nint main() {\n  auto node = std::make_shared<rclcpp::Node>(\"agg\");\n  node->declare_parameter(\"rate\", 1.0);\n  auto l = std::make_shared<p::ParamListener>(node, get_ns());\n}\n"),
            ("s/launch/a.launch.xml", "<launch>\n<arg name=\"verbose\" default=\"false\"/>\n<node pkg=\"s\" exec=\"agg\">\n  <param name=\"run_command\" value=\"ros2 launch x $(var verbose)\"/>\n</node>\n</launch>\n"),
        ]);
        assert!(
            d.iter()
                .any(|x| x.rule == "P003" && x.message.contains("`run_command`")),
            "{d:#?}"
        );
    }

    /// Values outside generate_parameter_library validators and descriptor ranges (P006).
    #[test]
    fn constraint_violations() {
        let run = |yaml: &str| {
            analyze_files(&[
                ("c/package.xml", "<package format=\"3\"><name>c</name></package>"),
                ("c/CMakeLists.txt", "project(c)
generate_parameter_library(ctrl_params src/ctrl.yaml)
add_executable(ctrl src/ctrl.cpp)
install(TARGETS ctrl DESTINATION lib/${PROJECT_NAME})
install(DIRECTORY launch config DESTINATION share/${PROJECT_NAME})
"),
                ("c/src/ctrl.yaml", "ctrl:
  wheel_separation:
    type: double
    default_value: 0.3
    validation:
      gt<>: [0.0]
  mode:
    type: string
    default_value: diff
    validation:
      one_of<>: [[\"diff\", \"ackermann\"]]
  joints:
    type: string_array
    default_value: [a]
    validation:
      not_empty<>: []
"),
                ("c/src/ctrl.cpp", "#include <rclcpp/rclcpp.hpp>
class C : public rclcpp::Node { public: C() : Node(\"ctrl\") {
  auto l = std::make_shared<ctrl::ParamListener>(this);
  rcl_interfaces::msg::ParameterDescriptor d;
  d.floating_point_range.resize(1);
  d.floating_point_range[0].from_value = 0.0;
  d.floating_point_range[0].to_value = 2.0;
  declare_parameter(\"max_speed\", 1.0, d);
} };
"),
                ("c/config/ctrl.yaml", yaml),
                ("c/launch/a.launch.py", "import os
from launch import LaunchDescription
from launch_ros.actions import Node
from ament_index_python.packages import get_package_share_directory
def generate_launch_description():
    return LaunchDescription([Node(package='c', executable='ctrl', parameters=[os.path.join(get_package_share_directory('c'), 'config', 'ctrl.yaml')])])
"),
            ])
        };
        let bad = run("ctrl:
  ros__parameters:
    wheel_separation: 0.0
    mode: tank
    joints: []
    max_speed: 3.5
");
        let p006: Vec<&str> = bad
            .iter()
            .filter(|x| x.rule == "P006")
            .map(|x| x.message.as_str())
            .collect();
        assert_eq!(p006.len(), 4, "{p006:#?}");
        assert!(
            p006.iter()
                .any(|m| m.contains("wheel_separation") && m.contains("`> 0`")),
            "{p006:#?}"
        );
        assert!(
            p006.iter()
                .any(|m| m.contains("mode") && m.contains("`diff`")),
            "{p006:#?}"
        );
        assert!(
            p006.iter()
                .any(|m| m.contains("max_speed") && m.contains("[0, 2]")),
            "{p006:#?}"
        );
        let good = run("ctrl:
  ros__parameters:
    wheel_separation: 0.5
    mode: ackermann
    joints: [left, right]
    max_speed: 2.0
");
        assert!(good.iter().all(|x| x.rule != "P006"), "{good:#?}");
    }

    /// Nav2 renamed `progress_checker_plugin` (Humble) to `progress_checker_plugins` (Jazzy).
    #[test]
    fn parameter_renamed_between_distros() {
        let run = |distro: &str| {
            let dir = tempfile::tempdir().unwrap();
            let w = |r: &str, t: &str| {
                let p = dir.path().join(r);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, t).unwrap();
            };
            w("bot/package.xml", "<package format=\"3\"><name>bot</name><exec_depend>nav2_controller</exec_depend></package>");
            w(
                "bot/config/nav2.yaml",
                "controller_server:
  ros__parameters:
    progress_checker_plugin: progress_checker
",
            );
            w("bot/launch/nav.launch.py", "import os
from launch import LaunchDescription
from launch_ros.actions import Node
from ament_index_python.packages import get_package_share_directory
def generate_launch_description():
    return LaunchDescription([Node(package='nav2_controller', executable='controller_server', name='controller_server', parameters=[os.path.join(get_package_share_directory('bot'), 'config', 'nav2.yaml')])])
");
            let opts = crate::engine::Options {
                distro: Some(distro.into()),
                ..Default::default()
            };
            crate::engine::analyze(dir.path(), &crate::config::Config::default(), &opts).diagnostics
        };
        let jazzy = run("jazzy");
        let p007 = jazzy
            .iter()
            .find(|x| x.rule == "P007")
            .unwrap_or_else(|| panic!("{jazzy:#?}"));
        assert!(
            !p007.message.contains("Humble") && !p007.message.contains("Jazzy"),
            "distro names must stay out of the message: {p007:#?}"
        );
        assert!(
            p007.help
                .as_deref()
                .is_some_and(|h| h.contains("Humble") && h.contains("progress_checker_plugins")),
            "{p007:#?}"
        );
        assert!(
            jazzy
                .iter()
                .all(|x| x.rule != "P003" || !x.message.contains("progress_checker_plugin")),
            "{jazzy:#?}"
        );
        let humble = run("humble");
        assert!(
            humble.iter().all(|x| x.rule != "P007"
                && !(x.rule == "P003" && x.message.contains("progress_checker"))),
            "{humble:#?}"
        );
    }

    /// A dependency's test code (nav2_util/test/test_node_utils.cpp declares `Foo`) must not
    /// count as declarations of the nodes that use the library.
    #[test]
    fn dependency_test_code_declares_nothing() {
        let d = analyze_files(&[
            ("util/package.xml", "<package format=\"3\"><name>util</name></package>"),
            ("util/src/lib.cpp", "#include <rclcpp/rclcpp.hpp>
void helper(rclcpp::Node & n) { n.declare_parameter(\"shared_rate\", 1.0); }
"),
            ("util/test/test_node_utils.cpp", "#include <rclcpp/rclcpp.hpp>
void t(rclcpp::Node & n) { n.declare_parameter(\"Foo\", 1); }
"),
            ("app/package.xml", "<package format=\"3\"><name>app</name><depend>util</depend></package>"),
            ("app/CMakeLists.txt", "project(app)
add_executable(n src/n.cpp)
install(TARGETS n DESTINATION lib/${PROJECT_NAME})
install(DIRECTORY launch DESTINATION share/${PROJECT_NAME})
"),
            ("app/src/n.cpp", "class N : public rclcpp::Node { public: N() : Node(\"n\") { declare_parameter(\"rate\", 1.0); } };
"),
            ("app/launch/a.launch.py", "from launch import LaunchDescription
from launch_ros.actions import Node
def generate_launch_description():
    return LaunchDescription([Node(package='app', executable='n', parameters=[{'Foo': 1, 'shared_rate': 2.0}])])
"),
        ]);
        let p003: Vec<&str> = d
            .iter()
            .filter(|x| x.rule == "P003")
            .map(|x| x.message.as_str())
            .collect();
        assert!(p003.iter().any(|m| m.contains("`Foo`")), "{p003:#?}");
        assert!(p003.iter().all(|m| !m.contains("shared_rate")), "{p003:#?}");
    }

    #[test]
    fn undeclared_and_type_mismatch() {
        let d = analyze_files(&[
            ("demo/package.xml", "<package format=\"3\"><name>demo</name><depend>rclcpp</depend></package>"),
            ("demo/CMakeLists.txt", "project(demo)\nadd_executable(talker src/talker.cpp)\ninstall(TARGETS talker DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch config DESTINATION share/${PROJECT_NAME})\n"),
            ("demo/src/talker.cpp", "#include <rclcpp/rclcpp.hpp>\nclass T : public rclcpp::Node { public: T() : Node(\"talker\") { declare_parameter(\"max_velocity\", 1.0); declare_parameter(\"frame\", std::string(\"map\")); } };\n"),
            ("demo/config/p.yaml", "talker:\n  ros__parameters:\n    max_velocty: 2.0\n    frame: map\n/robot/talker:\n  ros__parameters:\n    frame: x\n"),
            ("demo/config/q.yaml", "talker:\n  ros__parameters:\n    max_velocity: 2\n"),
            ("demo/launch/a.launch.py", "from launch import LaunchDescription\nfrom launch_ros.actions import Node\nimport os\nfrom ament_index_python.packages import get_package_share_directory\ndef generate_launch_description():\n    d = get_package_share_directory('demo')\n    return LaunchDescription([Node(package='demo', executable='talker', parameters=[os.path.join(d, 'config', 'p.yaml'), os.path.join(d, 'config', 'q.yaml')])])\n"),
        ]);
        let ids: Vec<&str> = d.iter().map(|x| x.rule).collect();
        assert!(ids.contains(&"P003"), "{d:#?}");
        assert!(ids.contains(&"P004"), "{d:#?}");
        assert!(ids.contains(&"P001"), "{d:#?}");
        let p004 = d.iter().find(|x| x.rule == "P004").unwrap();
        assert_eq!(p004.fix.as_ref().unwrap().edits[0].replacement, "2.0");
        let p003 = d.iter().find(|x| x.rule == "P003").unwrap();
        assert_eq!(
            p003.fix.as_ref().unwrap().edits[0].replacement,
            "max_velocity"
        );
    }

    #[test]
    fn namespaced_key_mismatch() {
        let d = analyze_files(&[
            ("demo/package.xml", "<package format=\"3\"><name>demo</name></package>"),
            ("demo/CMakeLists.txt", "project(demo)\nadd_executable(n src/n.cpp)\ninstall(TARGETS n DESTINATION lib/${PROJECT_NAME})\n"),
            ("demo/src/n.cpp", "class N : public rclcpp::Node { public: N() : Node(\"amcl\") { declare_parameter(\"a\", 1); } };\n"),
            ("demo/config/p.yaml", "amcl:\n  ros__parameters:\n    a: 2\n"),
            ("demo/launch/a.launch.xml", "<launch><node pkg=\"demo\" exec=\"n\" namespace=\"robot1\"><param from=\"$(find-pkg-share demo)/config/p.yaml\"/></node></launch>"),
        ]);
        let p001 = d.iter().find(|x| x.rule == "P001").expect("P001");
        assert!(p001.message.contains("/robot1/amcl"), "{}", p001.message);
    }
}
