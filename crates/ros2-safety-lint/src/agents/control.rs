//! ros2_control agent: controller configuration vs robot description.

use super::{Agent, Context};
use crate::catalog::{is_joint_param, CONTROLLERS};
use crate::diag::Diagnostic;
use crate::facts::*;
use crate::source::{FileId, Loc};
use crate::util::closest;
use crate::resolve::NodeInstance;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub struct ControlAgent;

struct Controller<'a> {
    name: String,
    ty: Option<(String, Loc, Option<String>)>,
    section: Option<&'a ParamSection>,
    listed_loc: Option<Loc>,
}

impl Agent for ControlAgent {
    fn id(&self) -> &'static str {
        "control"
    }
    fn title(&self) -> &'static str {
        "ros2_control (controllers vs hardware interfaces vs URDF)"
    }
    fn inputs(&self) -> &'static [crate::workspace::FileKind] {
        use crate::workspace::FileKind as K;
        // Robot descriptions, controller configuration, and the launch graph (spawners).
        &[K::Manifest, K::CMakeLists, K::SetupPy, K::SetupCfg, K::LaunchPy, K::LaunchXml, K::LaunchYaml, K::ParamYaml, K::Cpp, K::Python, K::Other, K::Urdf]
    }
    fn relevant(&self, cx: &Context) -> bool {
        cx.ix.robots.iter().any(|r| !r.control.is_empty())
            || cx.ix.param_files.values().any(|pf| {
                pf.sections
                    .iter()
                    .any(|s| s.node_key.ends_with("controller_manager"))
            })
            || cx.graph.instances().any(|(_, _, n)| is_spawner(n))
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        urdf_checks(cx, out);
        // Joint universe from every ros2_control block in the workspace.
        let mut joints: HashMap<&str, &ControlJoint> = HashMap::new();
        let mut partial = false;
        for r in &cx.ix.robots {
            partial |= r.partial;
            for b in &r.control {
                for j in &b.joints {
                    joints.insert(j.name.as_str(), j);
                }
            }
        }
        let workspace_types: HashSet<String> = cx
            .ix
            .packages
            .iter()
            .flat_map(|p| p.plugin_classes.iter().cloned())
            // Plugins exported by the installed ROS (any distro, custom controllers).
            .chain(cx.ix.installed.all_plugin_classes().map(|s| s.to_string()))
            .collect();
        for pf in cx.ix.param_files.values() {
            update_rates(pf, out);
            for c in controllers(pf) {
                check_controller(cx, &c, &joints, partial, &workspace_types, out);
                spec_constraints(cx, &c, out);
            }
        }
        spawner_checks(cx, out);
    }
}

/// C010: a controller's `update_rate` the controller manager cannot run it at (higher than,
/// or not a divisor of, the manager's `update_rate` in the same file).
fn update_rates(pf: &ParamFile, out: &mut Vec<Diagnostic>) {
    let rate = |e: &ParamEntry| match e.value {
        ParamValue::Int(i) if i > 0 => Some(i as f64),
        ParamValue::Double(d) if d > 0.0 => Some(d),
        _ => None,
    };
    let leaf = |k: &str| k.rsplit('/').next().unwrap_or(k).to_string();
    let Some(cm) = pf
        .sections
        .iter()
        .filter(|s| leaf(&s.node_key) == "controller_manager")
        .find_map(|s| s.params.iter().find(|e| e.name == "update_rate").and_then(rate))
    else {
        return;
    };
    for c in controllers(pf) {
        let Some(sec) = c.section else { continue };
        let Some(e) = sec.params.iter().find(|e| e.name == "update_rate") else { continue };
        let Some(r) = rate(e) else { continue };
        if r > cm {
            out.push(Diagnostic::new(
                "C010",
                e.value_loc,
                format!("controller `{}` asks for `update_rate` {r} Hz, but the controller manager runs at {cm} Hz; it can only run at {cm} Hz", c.name),
            ));
        } else if (cm / r).fract().abs() > 1e-9 {
            out.push(
                Diagnostic::new(
                    "C010",
                    e.value_loc,
                    format!("controller `{}`: `update_rate` {r} Hz does not divide the controller manager's {cm} Hz, so the controller runs at an uneven, lower rate", c.name),
                )
                .severity(crate::rules::Severity::Info),
            );
        }
    }
}

/// The parameter files a node instance loads (None if one cannot be resolved).
fn param_files_of(cx: &Context, n: &NodeInstance) -> Option<Vec<FileId>> {
    let mut v = Vec::new();
    for src in &n.decl.params {
        let LaunchParamSource::File { path, .. } = src else { continue };
        match crate::resolve::resolve_lval(path, &Default::default()).map(|p| crate::resolve::resolve_path(cx.ix, &p)) {
            Some(crate::resolve::PathRes::Found(f)) => v.push(f),
            _ => return None,
        }
    }
    Some(v)
}

/// C010 through the launch graph: the manager's rate (inline, from its parameter files, or
/// the 100 Hz default) against `update_rate` in controller sections of the manager's and the
/// spawners' (`--param-file`) parameter files.
fn graph_update_rates(cx: &Context, m: &NodeInstance, spawners: &[&NodeInstance], seen: &mut BTreeSet<Loc>, out: &mut Vec<Diagnostic>) {
    let num = |v: &ParamValue| match v {
        ParamValue::Int(i) if *i > 0 => Some(*i as f64),
        ParamValue::Double(d) if *d > 0.0 => Some(*d),
        _ => None,
    };
    let leaf = |k: &str| k.rsplit('/').next().unwrap_or(k).to_string();
    let Some(m_files) = param_files_of(cx, m) else { return };
    let inline = m.params.iter().rev().find(|b| b.name == "update_rate").and_then(|b| b.value.as_ref()).and_then(num);
    let from_file = m_files.iter().filter_map(|f| cx.ix.param_files.get(f)).flat_map(|pf| pf.sections.iter()).filter(|s| leaf(&s.node_key) == "controller_manager").find_map(|s| s.params.iter().find(|e| e.name == "update_rate").and_then(|e| num(&e.value)));
    // Sources unknown to us (e.g. an unresolved inline value) leave the rate unknown.
    if m.params.iter().any(|b| b.name == "update_rate" && b.value.as_ref().and_then(num).is_none()) {
        return;
    }
    let cm = inline.or(from_file).unwrap_or(100.0);
    let mut files = m_files;
    for sp in spawners {
        if let Some(Some(p)) = parse_spawner(sp).map(|s| s.param_file) {
            if let crate::resolve::PathRes::Found(f) = crate::resolve::resolve_path(cx.ix, &p) {
                files.push(f);
            }
        }
    }
    for pf in files.iter().filter_map(|f| cx.ix.param_files.get(f)) {
        // Files with their own controller_manager rate are checked file by file.
        if pf.sections.iter().any(|s| leaf(&s.node_key) == "controller_manager" && s.params.iter().any(|e| e.name == "update_rate")) {
            continue;
        }
        for sec in pf.sections.iter().filter(|s| leaf(&s.node_key) != "controller_manager") {
            let Some(e) = sec.params.iter().find(|e| e.name == "update_rate") else { continue };
            let Some(r) = num(&e.value) else { continue };
            if !seen.insert(e.value_loc) {
                continue;
            }
            let name = leaf(&sec.node_key);
            if r > cm {
                out.push(Diagnostic::new(
                    "C010",
                    e.value_loc,
                    format!("controller `{name}` asks for `update_rate` {r} Hz, but `{}` runs at {cm} Hz; it can only run at {cm} Hz", m.display_name()),
                ).related(m.decl.loc, "controller manager"));
            } else if (cm / r).fract().abs() > 1e-9 {
                out.push(
                    Diagnostic::new("C010", e.value_loc, format!("controller `{name}`: `update_rate` {r} Hz does not divide `{}`'s {cm} Hz, so it runs at an uneven, lower rate", m.display_name()))
                        .severity(crate::rules::Severity::Info),
                );
            }
        }
    }
}

/// C009: a controller's parameter violates a validator of its generate_parameter_library
/// spec (found through the controller type's package, e.g. `diff_drive_controller/…`).
fn spec_constraints(cx: &Context, c: &Controller, out: &mut Vec<Diagnostic>) {
    let (Some((ty, _, _)), Some(section)) = (&c.ty, c.section) else { return };
    let Some(pkg) = ty.split('/').next().and_then(|p| cx.ix.package_by_name(p)) else { return };
    let decls: Vec<&ParamDecl> = pkg.param_lib_specs.iter().filter_map(|s| cx.ix.param_specs.get(s)).flat_map(|(d, _)| d.iter()).collect();
    for e in &section.params {
        let Some(d) = decls.iter().find(|d| d.name.matches(&e.name)) else { continue };
        if let Some(why) = d.constraints.iter().find_map(|k| k.violated_by(&e.value)) {
            out.push(
                Diagnostic::new(
                    "C009",
                    e.value_loc,
                    format!("controller `{}` ({ty}): `{}` = {} violates {why}; the controller fails to configure", c.name, e.name, e.value.show()),
                )
                .related(d.loc, "validation declared here"),
            );
        }
    }
}

fn is_spawner(n: &NodeInstance) -> bool {
    n.pkg.as_deref() == Some("controller_manager") && matches!(n.exec.as_deref(), Some("spawner" | "spawner.py"))
}

fn is_manager(n: &NodeInstance) -> bool {
    n.pkg.as_deref() == Some("controller_manager") && n.exec.as_deref() == Some("ros2_control_node")
}

struct Spawn {
    names: Vec<String>,
    /// `--controller-type`: the spawner supplies the type itself.
    typed: bool,
    /// `--param-file <path>`: definitions come from this file.
    param_file: Option<String>,
}

/// Parses spawner arguments; None when an argument is unresolved or unrecognised.
fn parse_spawner(n: &NodeInstance) -> Option<Spawn> {
    const WITH_VALUE: &[&str] = &[
        "-c", "--controller-manager", "-p", "--param-file", "-t", "--controller-type", "-n", "--namespace",
        "--controller-manager-timeout", "--switch-timeout", "--service-call-timeout", "--controller-ros-args",
    ];
    const FLAGS: &[&str] = &["--load-only", "--inactive", "--stopped", "-u", "--unload-on-kill", "--activate-as-group"];
    let mut args = Vec::new();
    for a in &n.arguments {
        let a = a.as_ref()?;
        if a == "--ros-args" {
            break;
        }
        // One element per argument: XML `args="a b"` is split by the extractor; a Python
        // list element such as `"-r ~/cmd_vel:=/cmd_vel"` is one value.
        args.push(a.clone());
    }
    let mut s = Spawn { names: Vec::new(), typed: false, param_file: None };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if WITH_VALUE.contains(&a) {
            match a {
                "-t" | "--controller-type" => s.typed = true,
                "-p" | "--param-file" => s.param_file = Some(args.get(i + 1)?.clone()),
                _ => {}
            }
            i += 2;
        } else if FLAGS.contains(&a) {
            i += 1;
        } else if a.starts_with('-') {
            return None;
        } else {
            s.names.push(a.to_string());
            i += 1;
        }
    }
    Some(s)
}

/// Controllers defined (`<name>.type`) in the parameters a controller manager receives: bound
/// parameters, plus the controller_manager sections of its parameter files (the node's own
/// name comes from an installed package, so file sections are often not bound to it).
/// None when a parameter file cannot be resolved: then we do not know what is defined.
fn defined(cx: &Context, n: &NodeInstance) -> Option<BTreeMap<String, Loc>> {
    let mut defs: BTreeMap<String, Loc> = n
        .params
        .iter()
        .filter_map(|b| b.name.strip_suffix(".type").filter(|c| !c.contains('.')).map(|c| (c.to_string(), b.key_loc)))
        .collect();
    for src in &n.decl.params {
        let LaunchParamSource::File { path, .. } = src else { continue };
        let fid = match crate::resolve::resolve_lval(path, &Default::default()).map(|p| crate::resolve::resolve_path(cx.ix, &p)) {
            Some(crate::resolve::PathRes::Found(f)) => f,
            _ => return None,
        };
        let pf = cx.ix.param_files.get(&fid)?;
        for c in controllers(pf) {
            if let (Some(_), Some(loc)) = (&c.ty, c.listed_loc) {
                defs.entry(c.name).or_insert(loc);
            }
        }
    }
    Some(defs)
}

/// Controllers defined in a spawner `--param-file`: `<name>: ros__parameters: type: …`
/// sections, or `<name>.type` under `controller_manager`. None when the file is not known.
fn file_defs(cx: &Context, path: &str) -> Option<BTreeSet<String>> {
    let crate::resolve::PathRes::Found(fid) = crate::resolve::resolve_path(cx.ix, path) else { return None };
    let pf = cx.ix.param_files.get(&fid)?;
    let mut out: BTreeSet<String> = controllers(pf).into_iter().filter(|c| c.ty.is_some()).map(|c| c.name).collect();
    for sec in &pf.sections {
        let leaf = sec.node_key.rsplit('/').next().unwrap_or(&sec.node_key);
        if sec.params.iter().any(|e| e.name == "type") {
            out.insert(leaf.to_string());
        }
    }
    Some(out)
}

/// C007: a spawner names a controller its manager does not define.
/// C008: a controller is defined but no spawner in the workspace starts it.
fn spawner_checks(cx: &Context, out: &mut Vec<Diagnostic>) {
    let mut spawned_anywhere: BTreeSet<String> = BTreeSet::new();
    let mut all_known = true;
    for (_, _, n) in cx.graph.instances().filter(|(_, _, n)| is_spawner(n)) {
        match parse_spawner(n) {
            Some(s) => spawned_anywhere.extend(s.names),
            None => all_known = false,
        }
    }
    let mut rate_seen: BTreeSet<Loc> = BTreeSet::new();
    for root in &cx.graph.roots {
        let managers: Vec<&NodeInstance> = root.nodes.iter().filter(|n| is_manager(n)).collect();
        let [m] = managers.as_slice() else { continue };
        if root.partial {
            continue;
        }
        let Some(defs) = defined(cx, m) else { continue };
        let spawners: Vec<&NodeInstance> = root.nodes.iter().filter(|n| is_spawner(n)).collect();
        graph_update_rates(cx, m, &spawners, &mut rate_seen, out);
        for sp in &spawners {
            let Some(s) = parse_spawner(sp) else { continue };
            if s.typed {
                continue;
            }
            let from_file = match &s.param_file {
                Some(p) => match file_defs(cx, p) {
                    Some(f) => f,
                    None => continue,
                },
                // No definitions anywhere we can see: they come from elsewhere (simulator plugin).
                None if defs.is_empty() => continue,
                None => BTreeSet::new(),
            };
            for name in &s.names {
                if defs.contains_key(name.as_str()) || from_file.contains(name) {
                    continue;
                }
                let known: Vec<&str> = defs.keys().map(String::as_str).chain(from_file.iter().map(String::as_str)).collect();
                let mut d = Diagnostic::new(
                    "C007",
                    sp.decl.loc,
                    format!("spawner starts controller `{name}`, but neither `{}` nor the spawner's parameter file defines it (no `type`); the spawner fails", m.display_name()),
                );
                if let Some(c) = closest(name, known.iter().copied()) {
                    d = d.help(format!("Did you mean `{c}`?"));
                }
                out.push(d);
            }
        }
        if defs.is_empty() || spawners.is_empty() || !all_known || !root.external_includes.is_empty() {
            continue;
        }
        for (name, loc) in &defs {
            if !spawned_anywhere.contains(name) {
                out.push(
                    Diagnostic::new("C008", *loc, format!("controller `{name}` is configured for `{}`, but no spawner in the workspace starts it; it is never loaded", m.display_name()))
                        .help(format!("Add `{name}` to a `controller_manager` spawner, or remove its configuration.")),
                );
            }
        }
    }
}

fn urdf_checks(cx: &Context, out: &mut Vec<Diagnostic>) {
    for r in &cx.ix.robots {
        let urdf_joints: HashSet<&str> = r.joints.iter().map(|j| j.name.as_str()).collect();
        for b in &r.control {
            if b.hardware_plugin.is_none() {
                out.push(Diagnostic::new("C006", b.loc, format!("`<ros2_control name=\"{}\">` has no `<hardware><plugin>…</plugin></hardware>`", b.name)));
            }
            if r.partial {
                continue;
            }
            for j in &b.joints {
                if j.name.contains("${") || urdf_joints.contains(j.name.as_str()) {
                    continue;
                }
                let mut d = Diagnostic::new(
                    "C005",
                    j.loc,
                    format!(
                        "`<ros2_control>` joint `{}` does not exist in the robot description",
                        j.name
                    ),
                );
                if let Some(c) = closest(&j.name, urdf_joints.iter().copied()) {
                    d = d.help(format!("Did you mean `{c}`?"));
                }
                out.push(d);
            }
        }
    }
}

fn controllers(pf: &ParamFile) -> Vec<Controller<'_>> {
    let mut map: Vec<Controller> = Vec::new();
    let leaf = |k: &str| k.rsplit('/').next().unwrap_or(k).to_string();
    for s in &pf.sections {
        if leaf(&s.node_key) == "controller_manager" {
            for e in &s.params {
                if let (Some(name), ParamValue::Str(t)) = (e.name.strip_suffix(".type"), &e.value) {
                    map.push(Controller {
                        name: name.to_string(),
                        ty: Some((t.clone(), e.value_loc, e.raw.clone())),
                        section: None,
                        listed_loc: Some(e.key_loc),
                    });
                }
            }
        }
    }
    if map.is_empty()
        && !pf
            .sections
            .iter()
            .any(|s| s.params.iter().any(|e| e.name == "type"))
    {
        return map;
    }
    for s in &pf.sections {
        let name = leaf(&s.node_key);
        if name == "controller_manager" {
            continue;
        }
        let own_type = s
            .params
            .iter()
            .find(|e| e.name == "type")
            .and_then(|e| match &e.value {
                ParamValue::Str(t) if t.contains('/') => {
                    Some((t.clone(), e.value_loc, e.raw.clone()))
                }
                _ => None,
            });
        match map.iter_mut().find(|c| c.name == name) {
            Some(c) => {
                c.section = Some(s);
                if c.ty.is_none() {
                    c.ty = own_type;
                }
            }
            None => {
                let has_joint_params = s.params.iter().any(|e| is_joint_param(&e.name));
                if own_type.is_some() || (has_joint_params && !map.is_empty()) {
                    map.push(Controller {
                        name,
                        ty: own_type,
                        section: Some(s),
                        listed_loc: None,
                    });
                }
            }
        }
    }
    map
}

fn str_values(v: &ParamValue) -> Vec<String> {
    match v {
        ParamValue::Str(s) => vec![s.clone()],
        ParamValue::StrArray(a) => a.clone(),
        _ => Vec::new(),
    }
}

fn check_controller(
    cx: &Context,
    c: &Controller,
    joints: &HashMap<&str, &ControlJoint>,
    partial: bool,
    ws_types: &HashSet<String>,
    out: &mut Vec<Diagnostic>,
) {
    let _ = cx;
    // ------------------------------------------------------------- C003 type typo
    let known = |t: &str| CONTROLLERS.iter().any(|(k, _)| *k == t) || ws_types.contains(t);
    if let Some((t, loc, raw)) = &c.ty {
        if !known(t) {
            if let Some(s) = closest(t, CONTROLLERS.iter().map(|(k, _)| *k)) {
                let replacement = if raw.is_some() {
                    s.to_string()
                } else {
                    format!("\"{s}\"")
                };
                out.push(
                    Diagnostic::new("C003", *loc, format!("controller `{}` has type `{t}`, which is not a known controller plugin; did you mean `{s}`?", c.name))
                        .safe_fix(format!("Use `{s}`"), loc.file, loc.start, loc.end, replacement),
                );
            }
        }
    }
    // ------------------------------------------------------------- C004 consistency
    let ty_name = c.ty.as_ref().map(|t| t.0.as_str()).unwrap_or("");
    let needs_section = matches!(
        ty_name.split('/').next().unwrap_or(""),
        "joint_trajectory_controller"
            | "forward_command_controller"
            | "position_controllers"
            | "velocity_controllers"
            | "effort_controllers"
            | "diff_drive_controller"
    );
    match (c.section, c.listed_loc, &c.ty) {
        (None, Some(loc), _) if needs_section && ty_name.split('/').next_back() != Some("GripperActionController") => out.push(Diagnostic::new(
            "C004",
            loc,
            format!("controller `{}` ({ty_name}) is listed in controller_manager but has no `{}:` parameter section, so it cannot be configured", c.name, c.name),
        )),
        (Some(s), None, None) => out.push(Diagnostic::new(
            "C004",
            s.key_loc,
            format!("section `{}` configures joints like a controller but no `type` is set for it (in controller_manager or its own section)", s.node_key),
        )),
        _ => {}
    }
    // ------------------------------------------------------------- C001/C002 joints & interfaces
    let Some(s) = c.section else { return };
    if joints.is_empty() || partial {
        return;
    }
    let required_cmd: Vec<String> = match CONTROLLERS
        .iter()
        .find(|(k, _)| *k == ty_name)
        .and_then(|(_, i)| *i)
    {
        Some(i) => vec![i.to_string()],
        None => match ty_name {
            "forward_command_controller/ForwardCommandController" => s
                .params
                .iter()
                .find(|e| e.name == "interface_name")
                .map(|e| str_values(&e.value))
                .unwrap_or_default(),
            "joint_trajectory_controller/JointTrajectoryController" => s
                .params
                .iter()
                .find(|e| e.name == "command_interfaces")
                .map(|e| str_values(&e.value))
                .unwrap_or_default(),
            _ => Vec::new(),
        },
    };
    let required_state: Vec<String> =
        if ty_name == "joint_trajectory_controller/JointTrajectoryController" {
            s.params
                .iter()
                .find(|e| e.name == "state_interfaces")
                .map(|e| str_values(&e.value))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
    for e in s.params.iter().filter(|e| is_joint_param(&e.name)) {
        let names = str_values(&e.value);
        let missing: Vec<&String> = names
            .iter()
            .filter(|n| !n.contains('/') && !joints.contains_key(n.as_str()))
            .collect();
        if !missing.is_empty() {
            let mut d =
                Diagnostic::new(
                    "C001",
                    e.value_loc,
                    format!(
                    "controller `{}`: joint(s) {} are not declared in any `<ros2_control>` block",
                    c.name,
                    missing.iter().map(|m| format!("`{m}`")).collect::<Vec<_>>().join(", ")
                ),
                );
            if let Some(m) = missing.first() {
                if let Some(s) = closest(m, joints.keys().copied()) {
                    d = d.help(format!("Did you mean `{s}`?"));
                }
                if let Some(span) = super::value_span(cx.ix, e.value_loc, m) {
                    let used: HashSet<&str> = names.iter().map(|s| s.as_str()).collect();
                    let cands = joints
                        .keys()
                        .filter(|j| !used.contains(**j))
                        .map(|j| j.to_string())
                        .collect();
                    d = d.repair(span, m.as_str(), cands);
                }
            }
            out.push(d);
        }
        for n in names.iter().filter(|n| joints.contains_key(n.as_str())) {
            let j = joints[n.as_str()];
            for want in &required_cmd {
                if !j.command_interfaces.iter().any(|i| i == want) {
                    out.push(
                        Diagnostic::new(
                            "C002",
                            e.value_loc,
                            format!(
                                "controller `{}` ({ty_name}) needs a `{want}` command interface on joint `{n}`, but the hardware only exports [{}]",
                                c.name,
                                j.command_interfaces.join(", ")
                            ),
                        )
                        .related(j.loc, format!("joint `{n}` declared here")),
                    );
                }
            }
            for want in &required_state {
                if !j.state_interfaces.iter().any(|i| i == want) {
                    out.push(
                        Diagnostic::new(
                            "C002",
                            e.value_loc,
                            format!("controller `{}` needs a `{want}` state interface on joint `{n}`, but the hardware only exports [{}]", c.name, j.state_interfaces.join(", ")),
                        )
                        .related(j.loc, format!("joint `{n}` declared here")),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::testutil::analyze_files;

    #[test]
    fn controller_vs_urdf() {
        let urdf = r#"<robot name="r"><link name="base"/><link name="w"/>
<joint name="wheel_left" type="continuous"><parent link="base"/><child link="w"/></joint>
<ros2_control name="sys" type="system"><hardware><plugin>mock_components/GenericSystem</plugin></hardware>
<joint name="wheel_left"><command_interface name="position"/><state_interface name="position"/></joint>
<joint name="ghost"><command_interface name="position"/></joint>
</ros2_control></robot>"#;
        let yaml = "controller_manager:\n  ros__parameters:\n    vel_ctrl:\n      type: velocity_controllers/JointGroupVelocityController\n    jtc:\n      type: joint_trajectory_controler/JointTrajectoryController\nvel_ctrl:\n  ros__parameters:\n    joints: [wheel_left, wheel_rigth]\n";
        let d = analyze_files(&[
            (
                "p/package.xml",
                "<package format=\"3\"><name>p</name></package>",
            ),
            ("p/urdf/r.urdf", urdf),
            ("p/config/c.yaml", yaml),
        ]);
        let ids: Vec<&str> = d.iter().map(|x| x.rule).collect();
        assert!(ids.contains(&"C001"), "{d:#?}");
        assert!(ids.contains(&"C002"), "{d:#?}");
        assert!(ids.contains(&"C003"), "{d:#?}");
        assert!(ids.contains(&"C005"), "{d:#?}");
    }

    fn spawner_ws(spawned: &str) -> Vec<(&'static str, String)> {
        vec![
            ("p/package.xml", "<package format=\"3\"><name>p</name></package>".into()),
            ("p/config/c.yaml", "controller_manager:\n  ros__parameters:\n    update_rate: 100\n    joint_state_broadcaster:\n      type: joint_state_broadcaster/JointStateBroadcaster\n    diff_drive_controller:\n      type: diff_drive_controller/DiffDriveController\n".into()),
            ("p/launch/a.launch.py", format!("import os\nfrom launch import LaunchDescription\nfrom launch_ros.actions import Node\nfrom ament_index_python.packages import get_package_share_directory\ndef generate_launch_description():\n    cfg = os.path.join(get_package_share_directory('p'), 'config', 'c.yaml')\n    return LaunchDescription([\n        Node(package='controller_manager', executable='ros2_control_node', parameters=[cfg]),\n{spawned}    ])\n")),
        ]
    }

    fn run(spawned: &str) -> Vec<crate::diag::Diagnostic> {
        let ws = spawner_ws(spawned);
        let refs: Vec<(&str, &str)> = ws.iter().map(|(a, b)| (*a, b.as_str())).collect();
        analyze_files(&refs)
    }

    #[test]
    fn spawned_vs_defined_controllers() {
        let d = run("        Node(package='controller_manager', executable='spawner', arguments=['joint_state_broadcaster', '--controller-manager', '/controller_manager']),\n        Node(package='controller_manager', executable='spawner', arguments=['diff_drive_controler']),\n");
        let c007 = d.iter().find(|x| x.rule == "C007").expect("C007");
        assert!(c007.message.contains("diff_drive_controler") && c007.help.as_deref().is_some_and(|h| h.contains("diff_drive_controller")), "{c007:#?}");
        let c008: Vec<_> = d.iter().filter(|x| x.rule == "C008").collect();
        assert!(c008.len() == 1 && c008[0].message.contains("`diff_drive_controller`"), "{c008:#?}");
    }

    #[test]
    fn all_spawned_or_unknown_spawners_are_silent() {
        let ok = run("        Node(package='controller_manager', executable='spawner', arguments=['joint_state_broadcaster', 'diff_drive_controller', '--inactive', '--controller-ros-args', '-r ~/cmd_vel:=/cmd_vel']),\n");
        assert!(ok.iter().all(|x| x.rule != "C007" && x.rule != "C008"), "{ok:#?}");
        // A spawner with an unknown flag: we cannot tell what it starts, so no C008.
        let unk = run("        Node(package='controller_manager', executable='spawner', arguments=['joint_state_broadcaster', '--some-new-flag']),\n");
        assert!(unk.iter().all(|x| x.rule != "C008"), "{unk:#?}");
        // --param-file: the spawner brings its own definition.
        let pf = run("        Node(package='controller_manager', executable='spawner', arguments=['joint_state_broadcaster', 'diff_drive_controller', 'extra_ctrl', '--param-file', '/x.yaml']),\n");
        assert!(pf.iter().all(|x| x.rule != "C007"), "{pf:#?}");
    }

    /// ros2_control_demos style: the manager gets no controllers, each spawner brings a
    /// `--param-file` with `<name>: ros__parameters: type:` sections.
    #[test]
    fn spawner_param_file_definitions() {
        let ws = |names: &str| {
            vec![
                ("q/package.xml", "<package format=\"3\"><name>q</name></package>".to_string()),
                ("q/config/ctrl.yaml", "joint_state_broadcaster:
  ros__parameters:
    type: joint_state_broadcaster/JointStateBroadcaster
diffbot_base_controller:
  ros__parameters:
    type: diff_drive_controller/DiffDriveController
".to_string()),
                ("q/launch/a.launch.py", format!("import os
from launch import LaunchDescription
from launch_ros.actions import Node
from ament_index_python.packages import get_package_share_directory
def generate_launch_description():
    cfg = os.path.join(get_package_share_directory('q'), 'config', 'ctrl.yaml')
    return LaunchDescription([
        Node(package='controller_manager', executable='ros2_control_node', parameters=[{{'update_rate': 10}}]),
        Node(package='controller_manager', executable='spawner', arguments=[{names}, '--param-file', cfg]),
    ])
")),
            ]
        };
        let run = |names: &str| {
            let w = ws(names);
            let r: Vec<(&str, &str)> = w.iter().map(|(a, b)| (*a, b.as_str())).collect();
            analyze_files(&r)
        };
        let bad = run("'joint_state_broadcaster', 'diffbot_base_controler'");
        let c007 = bad.iter().find(|x| x.rule == "C007").expect("C007");
        assert!(c007.help.as_deref().is_some_and(|h| h.contains("diffbot_base_controller")), "{c007:#?}");
        let good = run("'joint_state_broadcaster', 'diffbot_base_controller'");
        assert!(good.iter().all(|x| x.rule != "C007" && x.rule != "C008"), "{good:#?}");
    }

    /// Controller section values checked against the controller package's
    /// generate_parameter_library spec (ros2_controllers style, flow mappings).
    #[test]
    fn controller_spec_validation() {
        let run = |sep: &str| {
            let cfg = format!("controller_manager:
  ros__parameters:
    base:
      type: diff_drive_controller/DiffDriveController
base:
  ros__parameters:
    wheel_separation: {sep}
    left_wheel_names: [l]
");
            analyze_files(&[
                ("diff_drive_controller/package.xml", "<package format=\"3\"><name>diff_drive_controller</name></package>"),
                ("diff_drive_controller/CMakeLists.txt", "project(diff_drive_controller)
generate_parameter_library(ddc_params src/p.yaml)
"),
                ("diff_drive_controller/src/p.yaml", "diff_drive_controller:
  wheel_separation: {
    type: double,
    default_value: 0.0,
    validation: {
      gt<>: [0.0]
    }
  }
  left_wheel_names: {
    type: string_array,
    default_value: [],
    validation: {
      not_empty<>: []
    }
  }
"),
                ("robot/package.xml", "<package format=\"3\"><name>robot</name></package>"),
                ("robot/config/c.yaml", &cfg),
            ])
        };
        let bad = run("-0.1");
        assert!(bad.iter().any(|x| x.rule == "C009" && x.message.contains("wheel_separation") && x.message.contains("`> 0`")), "{bad:#?}");
        let good = run("0.3");
        assert!(good.iter().all(|x| x.rule != "C009"), "{good:#?}");
    }

    #[test]
    fn controller_update_rates() {
        let d = analyze_files(&[
            ("p/package.xml", "<package format=\"3\"><name>p</name></package>"),
            ("p/config/c.yaml", "controller_manager:\n  ros__parameters:\n    update_rate: 100\n    fast:\n      type: joint_state_broadcaster/JointStateBroadcaster\n    odd:\n      type: joint_state_broadcaster/JointStateBroadcaster\n    ok:\n      type: joint_state_broadcaster/JointStateBroadcaster\nfast:\n  ros__parameters:\n    update_rate: 500\nodd:\n  ros__parameters:\n    update_rate: 30\nok:\n  ros__parameters:\n    update_rate: 50\n"),
        ]);
        let c010: Vec<&str> = d.iter().filter(|x| x.rule == "C010").map(|x| x.message.as_str()).collect();
        assert_eq!(c010.len(), 2, "{c010:#?}");
        assert!(c010.iter().any(|m| m.contains("`fast`")) && c010.iter().any(|m| m.contains("`odd`")), "{c010:#?}");
    }
}
