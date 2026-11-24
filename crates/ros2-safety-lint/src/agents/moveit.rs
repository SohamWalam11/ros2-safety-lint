//! MoveIt agent: a MoveIt configuration package against itself, the robot description and
//! the ros2_control controllers.
//!
//! - V001 a `kinematics.yaml` group that is not a planning group of the SRDF: MoveIt never
//!   loads its solver ("No kinematics plugins defined");
//! - V002 a `joint_limits.yaml` joint that the robot description does not have: its limits
//!   are never used;
//! - V003 a joint limit MoveIt reads, written as an integer: it logs "expected [double] got
//!   [integer]" and skips the joint's remaining limits (a velocity, acceleration or jerk
//!   limit is read only when its `has_*_limits` flag is true);
//! - V004 an SRDF group, chain, collision pair or end effector naming a joint or link the
//!   robot description does not have: the robot model fails to load;
//! - V005 a `moveit_controllers.yaml` controller that no ros2_control configuration in the
//!   workspace defines: trajectory execution has nothing to send to.
//!
//! Robot-description checks (V002, V004) run only when every robot description in the
//! workspace was fully evaluated; a name counts as known if any of them has it (several
//! robots in one workspace cannot then produce a false report). An `.srdf.xacro` is read
//! only when it uses no xacro constructs. V004 checks only SRDFs of MoveIt configuration
//! packages (a MoveIt dependency or MoveIt YAML in the package): Pinocchio and other tools
//! read SRDFs with conventions of their own. Names filled in later (`${…}`, `$(…)`) are
//! never checked.

use super::{Agent, Context};
use crate::diag::Diagnostic;
use crate::extract::yaml::{self, YNode};
use crate::source::{FileId, Loc};
use crate::util::closest;
use crate::workspace::FileKind;
use std::collections::{BTreeMap, BTreeSet, HashSet};

pub struct MoveItAgent;

/// Limits MoveIt's robot model loader reads as doubles, with the flag that must be true for
/// it to read them (moveit_ros/planning/robot_model_loader).
const LIMIT_DOUBLES: &[(&str, Option<&str>)] = &[
    ("max_position", None),
    ("min_position", None),
    ("max_velocity", Some("has_velocity_limits")),
    ("max_acceleration", Some("has_acceleration_limits")),
    ("max_jerk", Some("has_jerk_limits")),
];

impl Agent for MoveItAgent {
    fn id(&self) -> &'static str {
        "moveit"
    }
    fn title(&self) -> &'static str {
        "MoveIt configuration (SRDF, kinematics, joint limits, controllers)"
    }
    fn inputs(&self) -> &'static [FileKind] {
        &[FileKind::MoveIt, FileKind::Urdf, FileKind::ParamYaml, FileKind::Manifest]
    }
    fn relevant(&self, cx: &Context) -> bool {
        cx.ix.kinds.contains(&FileKind::MoveIt)
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        let files: Vec<FileId> = (0..cx.ix.kinds.len())
            .filter(|i| cx.ix.kinds[*i] == FileKind::MoveIt && !cx.ix.external_files.contains(i))
            .collect();
        let robot = robot_names(cx);
        // SRDF files are also read by Pinocchio and other tools with their own conventions
        // (a `root_joint` the URDF does not have, partial models). Only packages that are
        // MoveIt configurations get the SRDF name check: those that depend on MoveIt or
        // hold MoveIt YAML next to the SRDF.
        let mut moveit_dirs: HashSet<String> = cx
            .ix
            .packages
            .iter()
            .filter(|p| p.deps.iter().any(|d| d.name.starts_with("moveit")))
            .map(|p| p.dir.clone())
            .collect();
        moveit_dirs.extend(files.iter().filter(|f| !cx.ix.file(**f).rel_path.contains(".srdf")).map(|f| package_dir(cx, *f)));
        // SRDF facts per package directory.
        let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for &f in &files {
            let file = cx.ix.file(f);
            if !file.rel_path.contains(".srdf") {
                continue;
            }
            if file.rel_path.ends_with(".xacro") && (file.text.contains("xacro:") || file.text.contains("${")) {
                continue;
            }
            let Ok(doc) = roxmltree::Document::parse(&file.text) else { continue };
            let dir = package_dir(cx, f);
            let is_moveit = moveit_dirs.contains(&dir);
            let g = groups.entry(dir).or_default();
            for n in doc.root_element().children().filter(|n| n.has_tag_name("group")) {
                if let Some(name) = n.attribute("name") {
                    g.insert(name.to_string());
                }
            }
            if let (Some((joints, links)), true) = (&robot, is_moveit) {
                // Virtual joints are declared by the SRDF itself and are valid group members.
                let mut joints = joints.clone();
                joints.extend(
                    doc.root_element()
                        .children()
                        .filter(|n| n.has_tag_name("virtual_joint"))
                        .filter_map(|n| n.attribute("name").map(str::to_string)),
                );
                srdf_names(f, &doc, &joints, links, out);
            }
        }
        let controllers = ros2_control_controllers(cx);
        for &f in &files {
            let file = cx.ix.file(f);
            let name = file.rel_path.rsplit('/').next().unwrap_or("");
            let Ok(docs) = yaml::parse(&file.text) else { continue };
            let Some(doc) = docs.first() else { continue };
            match name {
                "kinematics.yaml" => {
                    if let Some(g) = groups.get(&package_dir(cx, f)) {
                        kinematics_groups(f, doc, g, out);
                    }
                }
                "joint_limits.yaml" => joint_limits(f, doc, robot.as_ref().map(|r| &r.0), out),
                "moveit_controllers.yaml" => {
                    if !controllers.is_empty() {
                        moveit_controllers(f, doc, &controllers, out);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Joint and link names of the workspace's robot descriptions; None when any is partial
/// (unresolved xacro) or there is none.
fn robot_names(cx: &Context) -> Option<(HashSet<String>, HashSet<String>)> {
    if cx.ix.robots.is_empty() || cx.ix.robots.iter().any(|r| r.partial) {
        return None;
    }
    let joints = cx.ix.robots.iter().flat_map(|r| r.joints.iter().map(|j| j.name.clone())).collect();
    let links = cx.ix.robots.iter().flat_map(|r| r.links.iter().cloned()).collect();
    Some((joints, links))
}

/// A name that is filled in later (xacro `${…}`, launch `$(…)`), so it cannot be checked.
fn templated(name: &str) -> bool {
    name.contains("${") || name.contains("$(")
}

/// The directory of the package a file belongs to (its MoveIt configuration package).
fn package_dir(cx: &Context, f: FileId) -> String {
    cx.ix.package_of(f).map(|p| p.dir.clone()).unwrap_or_default()
}

fn attr_loc(f: FileId, n: &roxmltree::Node, attr: &str) -> Loc {
    n.attribute_node(attr).map_or(Loc::new(f, n.range().start, n.range().end), |a| {
        let r = a.range_value();
        Loc::new(f, r.start, r.end)
    })
}

/// V004: names in the SRDF that the robot description does not have.
fn srdf_names(f: FileId, doc: &roxmltree::Document, joints: &HashSet<String>, links: &HashSet<String>, out: &mut Vec<Diagnostic>) {
    let mut check = |n: &roxmltree::Node, attr: &str, is_joint: bool, what: &str| {
        let Some(v) = n.attribute(attr) else { return };
        let known = if is_joint { joints } else { links };
        if known.contains(v) || templated(v) {
            return;
        }
        let kind = if is_joint { "joint" } else { "link" };
        let mut d = Diagnostic::new(
            "V004",
            attr_loc(f, n, attr),
            format!("{what} names {kind} `{v}`, which the robot description does not have; MoveIt fails to load the robot model"),
        );
        if let Some(c) = closest(v, known.iter().map(String::as_str)) {
            d = d.help(format!("Did you mean `{c}`?"));
        }
        out.push(d);
    };
    for n in doc.root_element().descendants() {
        let group = n.ancestors().find(|a| a.has_tag_name("group")).and_then(|g| g.attribute("name")).unwrap_or("");
        match n.tag_name().name() {
            "joint" if n.parent().is_some_and(|p| p.has_tag_name("group")) => check(&n, "name", true, &format!("group `{group}`")),
            "link" if n.parent().is_some_and(|p| p.has_tag_name("group")) => check(&n, "name", false, &format!("group `{group}`")),
            "chain" => {
                check(&n, "base_link", false, &format!("the chain of group `{group}`"));
                check(&n, "tip_link", false, &format!("the chain of group `{group}`"));
            }
            "disable_collisions" => {
                check(&n, "link1", false, "a disabled collision pair");
                check(&n, "link2", false, "a disabled collision pair");
            }
            "end_effector" => check(&n, "parent_link", false, &format!("end effector `{}`", n.attribute("name").unwrap_or(""))),
            "passive_joint" => check(&n, "name", true, "a passive joint"),
            _ => {}
        }
    }
}

/// V001: kinematics.yaml groups that are not SRDF planning groups.
fn kinematics_groups(f: FileId, doc: &YNode, groups: &BTreeSet<String>, out: &mut Vec<Diagnostic>) {
    // MoveIt 2 accepts the groups at the top level or under `robot_description_kinematics:`.
    let root = doc.get("robot_description_kinematics").unwrap_or(doc);
    for (k, v) in root.entries() {
        let Some(name) = k.as_str() else { continue };
        if !v.is_map() || v.get("kinematics_solver").is_none() || groups.contains(name) || templated(name) {
            continue;
        }
        let mut d = Diagnostic::new(
            "V001",
            Loc::new(f, k.start(), k.end()),
            format!("`{name}` is not a planning group of the SRDF, so MoveIt never loads this kinematics solver (\"No kinematics plugins defined\")"),
        );
        match closest(name, groups.iter().map(String::as_str)) {
            Some(c) => d = d.help(format!("Did you mean `{c}`?")).safe_fix(format!("Rename the group to `{c}`"), f, k.start(), k.end(), c),
            None => {
                let all: Vec<&str> = groups.iter().map(String::as_str).collect();
                d = d.help(format!("SRDF groups: {}", all.join(", ")));
            }
        }
        out.push(d);
    }
}

/// V002 and V003: joint_limits.yaml entries.
fn joint_limits(f: FileId, doc: &YNode, joints: Option<&HashSet<String>>, out: &mut Vec<Diagnostic>) {
    let Some(limits) = doc.get("joint_limits") else { return };
    for (k, v) in limits.entries() {
        let Some(joint) = k.as_str() else { continue };
        if let Some(j) = joints {
            if !j.contains(joint) && !templated(joint) {
                let mut d = Diagnostic::new(
                    "V002",
                    Loc::new(f, k.start(), k.end()),
                    format!("joint `{joint}` is not in the robot description; its limits are never used"),
                );
                if let Some(c) = closest(joint, j.iter().map(String::as_str)) {
                    d = d.help(format!("Did you mean `{c}`?"));
                }
                out.push(d);
            }
        }
        for (lk, lv) in v.entries() {
            let (Some(field), YNode::Scalar { value, quoted: false, start, end }) = (lk.as_str(), lv) else { continue };
            let Some((_, flag)) = LIMIT_DOUBLES.iter().find(|(n, _)| *n == field) else { continue };
            let read = flag.is_none_or(|fl| v.get(fl).and_then(YNode::as_str).is_some_and(|b| b.eq_ignore_ascii_case("true")));
            if read && value.parse::<i64>().is_ok() {
                out.push(
                    Diagnostic::new(
                        "V003",
                        Loc::new(f, *start, *end),
                        format!("`{joint}.{field}` is written as the integer `{value}`; MoveIt reads it as a double, logs \"expected [double] got [integer]\" and skips this and the joint's later limits"),
                    )
                    .safe_fix(format!("Write `{value}.0`"), f, *start, *end, format!("{value}.0")),
                );
            }
        }
    }
}

/// Controller names that ros2_control configurations in the workspace define
/// (`<…>controller_manager: ros__parameters: <name>: type: …`).
fn ros2_control_controllers(cx: &Context) -> HashSet<String> {
    let mut out = HashSet::new();
    for pf in cx.ix.param_files.values() {
        for s in &pf.sections {
            // Robots may run several managers (`manipulator_controller_manager`, …).
            if !s.node_key.rsplit('/').next().is_some_and(|n| n.ends_with("controller_manager")) {
                continue;
            }
            for e in &s.params {
                if let Some(c) = e.name.strip_suffix(".type") {
                    out.insert(c.to_string());
                }
            }
        }
    }
    out
}

/// V005: moveit_controllers.yaml controllers that ros2_control does not define.
fn moveit_controllers(f: FileId, doc: &YNode, defined: &HashSet<String>, out: &mut Vec<Diagnostic>) {
    let Some(mgr) = doc.get("moveit_simple_controller_manager") else { return };
    let Some(names) = mgr.get("controller_names") else { return };
    for n in names.items() {
        let Some(name) = n.as_str() else { continue };
        if defined.contains(name) || templated(name) {
            continue;
        }
        let mut d = Diagnostic::new(
            "V005",
            Loc::new(f, n.start(), n.end()),
            format!("MoveIt sends trajectories to controller `{name}`, but no ros2_control configuration in the workspace defines it"),
        );
        if let Some(c) = closest(name, defined.iter().map(String::as_str)) {
            d = d.help(format!("Did you mean `{c}`?"));
        }
        out.push(d);
    }
}

#[cfg(test)]
mod tests {
    use crate::testutil::analyze_files;

    const URDF: (&str, &str) = (
        "arm_description/urdf/arm.urdf",
        "<robot name=\"arm\"><link name=\"base_link\"/><link name=\"link1\"/><link name=\"tool0\"/>\n<joint name=\"joint1\" type=\"revolute\"><parent link=\"base_link\"/><child link=\"link1\"/></joint>\n<joint name=\"joint2\" type=\"fixed\"><parent link=\"link1\"/><child link=\"tool0\"/></joint></robot>",
    );
    const SRDF: &str = "<robot name=\"arm\">\n<virtual_joint name=\"world_joint\" type=\"fixed\" parent_frame=\"world\" child_link=\"base_link\"/>\n<group name=\"manipulator\"><joint name=\"world_joint\"/><chain base_link=\"base_link\" tip_link=\"tool0\"/></group>\n<group name=\"gripper\"><link name=\"tool0\"/><joint name=\"joint9\"/></group>\n<disable_collisions link1=\"base_link\" link2=\"link_one\" reason=\"Adjacent\"/>\n</robot>";
    const CTRL: (&str, &str) = (
        "arm_bringup/config/ros2_controllers.yaml",
        "controller_manager:\n  ros__parameters:\n    arm_controller:\n      type: joint_trajectory_controller/JointTrajectoryController\n",
    );

    fn cfg(kin: &str, limits: &str, ctrls: &str) -> Vec<crate::diag::Diagnostic> {
        analyze_files(&[
            ("arm_description/package.xml", "<package format=\"3\"><name>arm_description</name></package>"),
            URDF,
            ("arm_moveit_config/package.xml", "<package format=\"3\"><name>arm_moveit_config</name></package>"),
            ("arm_moveit_config/config/arm.srdf", SRDF),
            ("arm_moveit_config/config/kinematics.yaml", kin),
            ("arm_moveit_config/config/joint_limits.yaml", limits),
            ("arm_moveit_config/config/moveit_controllers.yaml", ctrls),
            ("arm_bringup/package.xml", "<package format=\"3\"><name>arm_bringup</name></package>"),
            CTRL,
        ])
    }

    #[test]
    fn moveit_checks() {
        let d = cfg(
            "manipulater:\n  kinematics_solver: kdl_kinematics_plugin/KDLKinematicsPlugin\nhand_group:\n  kinematics_solver: kdl_kinematics_plugin/KDLKinematicsPlugin\n",
            "joint_limits:\n  joint1:\n    has_velocity_limits: true\n    max_velocity: 2\n  jiont1:\n    max_velocity: 1.0\n",
            "moveit_simple_controller_manager:\n  controller_names:\n    - arm_controler\n",
        );
        let by = |r: &str| d.iter().filter(|x| x.rule == r).map(|x| x.message.clone()).collect::<Vec<_>>();
        assert_eq!(by("V001").len(), 2, "{d:#?}");
        assert_eq!(by("V002").len(), 1, "{d:#?}");
        assert!(by("V002")[0].contains("`jiont1`"));
        assert_eq!(by("V003").len(), 1, "{d:#?}");
        assert!(by("V003")[0].contains("`joint1.max_velocity`"));
        let v004 = by("V004");
        assert_eq!(v004.len(), 2, "{v004:#?}");
        assert!(v004.iter().any(|m| m.contains("`joint9`")) && v004.iter().any(|m| m.contains("`link_one`")));
        assert!(by("V005").iter().any(|m| m.contains("`arm_controler`")), "{d:#?}");
        // A close typo gets a safe fix; a name close to no group lists the groups instead.
        let v001 = |n: &str| d.iter().find(|x| x.rule == "V001" && x.message.contains(n)).unwrap();
        assert!(v001("`manipulater`").fix.is_some());
        assert!(v001("`hand_group`").fix.is_none() && v001("`hand_group`").help.as_deref().is_some_and(|h| h.contains("gripper, manipulator")));
        assert!(d.iter().filter(|x| x.rule == "V003").all(|x| x.fix.is_some()));
    }

    #[test]
    fn consistent_configuration_is_silent() {
        let d = cfg(
            "manipulator:\n  kinematics_solver: kdl_kinematics_plugin/KDLKinematicsPlugin\n",
            // A xacro-templated name is filled in later and cannot be checked.
            // An integer limit whose has_*_limits flag is false is never read.
            "joint_limits:\n  joint1:\n    has_velocity_limits: true\n    max_velocity: 2.0\n    has_acceleration_limits: false\n    max_acceleration: 0\n  ${prefix}joint1:\n    max_velocity: 1.0\n",
            "moveit_simple_controller_manager:\n  controller_names:\n    - arm_controller\n",
        );
        let ours: Vec<_> = d.iter().filter(|x| x.rule.starts_with('V') && !x.message.contains("joint9") && !x.message.contains("link_one")).collect();
        assert!(ours.is_empty(), "{ours:#?}");
    }

    #[test]
    fn srdf_outside_a_moveit_package_is_not_checked() {
        // Pinocchio model packages ship SRDFs with a `root_joint` the URDF does not have.
        let d = analyze_files(&[
            ("arm_description/package.xml", "<package format=\"3\"><name>arm_description</name></package>"),
            URDF,
            ("arm_description/srdf/arm.srdf", "<robot name=\"arm\"><group name=\"whole_body\"><joint name=\"root_joint\"/></group></robot>"),
        ]);
        assert!(!d.iter().any(|x| x.rule == "V004"), "{d:#?}");
    }
}
