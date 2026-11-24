//! TF agent: the transform tree a launch graph builds.
//!
//! Frames come from the robot description (URDF joints: parent → child) and from
//! `static_transform_publisher` nodes. Frames that nodes publish from code at runtime are
//! not visible statically, so this agent only reports what is certain:
//! - F001 a frame with two different parents (tf2 rejects or flips between them);
//! - F002 a `static_transform_publisher` whose arguments it cannot accept (exits at startup);
//! - F003 a frame the node looks up starting with `/` (tf2 rejects it in lookups);
//! - F004 a frame parameter that is a near-typo of a known frame.

use super::{value_span, Agent, Context};
use crate::catalog::{NAV2_FRAME_PARAMS, WORLD_FRAMES};
use crate::diag::{Diagnostic, Fix, FixSafety, TextEdit};
use crate::facts::ParamValue;
use crate::resolve::{NodeInstance, RootGraph};
use crate::source::Loc;
use crate::util::closest;
use std::collections::{BTreeMap, BTreeSet};

pub struct TfAgent;

const STP: &str = "static_transform_publisher";

/// Flags of the ROS 2 (Foxy+) `static_transform_publisher`; each takes a value.
const STP_FLAGS: &[&str] = &[
    "--x",
    "--y",
    "--z",
    "--roll",
    "--pitch",
    "--yaw",
    "--qx",
    "--qy",
    "--qz",
    "--qw",
    "--frame-id",
    "--child-frame-id",
];

/// Parameters naming a frame the node *looks up* in tf (Nav2, robot_localization, common
/// conventions). tf2 strips a leading `/` from transforms it receives, but rejects it in
/// lookups, so only these are reported by F003. Publisher-side ids (`frame_id`,
/// `child_frame_id`, static transform arguments) work with a slash.
const LOOKUP_FRAME_PARAMS: &[&str] = &[
    "global_frame",
    "robot_base_frame",
    "base_frame_id",
    "odom_frame_id",
    "global_frame_id",
    "base_frame",
    "odom_frame",
    "map_frame",
    "world_frame",
    "base_link_frame",
    "target_frame",
    "fixed_frame",
];

/// Parameter names that hold a frame id: `frame_id`, `*_frame`, `*_frame_id`.
fn is_frame_param(name: &str) -> bool {
    let base = name.rsplit('.').next().unwrap_or(name);
    base == "frame_id" || base.ends_with("_frame") || base.ends_with("_frame_id")
}

enum StpArgs {
    /// (parent, child)
    Ok(String, String),
    Invalid(String),
    /// Unresolved or unrecognised arguments: say nothing.
    Unknown,
}

fn parse_stp(n: &NodeInstance) -> StpArgs {
    let mut args = Vec::new();
    for a in &n.arguments {
        let Some(a) = a else { return StpArgs::Unknown };
        if a == "--ros-args" {
            break;
        }
        // One element per argument: XML `args="a b"` is split by the extractor; a Python
        // list element such as `"-r ~/cmd_vel:=/cmd_vel"` is one value.
        args.push(a.clone());
    }
    if args.iter().any(|a| a.starts_with("--")) {
        let (mut parent, mut child) = (None, None);
        let mut i = 0;
        while i < args.len() {
            let flag = args[i].as_str();
            if !STP_FLAGS.contains(&flag) {
                return StpArgs::Unknown; // e.g. extra ros arguments: not ours to judge
            }
            let Some(v) = args.get(i + 1) else {
                return StpArgs::Invalid(format!("`{flag}` has no value"));
            };
            match flag {
                "--frame-id" => parent = Some(v.clone()),
                "--child-frame-id" => child = Some(v.clone()),
                _ if v.parse::<f64>().is_err() => {
                    return StpArgs::Invalid(format!("`{flag} {v}` is not a number"))
                }
                _ => {}
            }
            i += 2;
        }
        return match (parent, child) {
            (Some(p), Some(c)) => StpArgs::Ok(p, c),
            _ => StpArgs::Invalid("both `--frame-id` and `--child-frame-id` are required".into()),
        };
    }
    // Deprecated positional form: x y z yaw pitch roll parent child | x y z qx qy qz qw parent child.
    let nums = args.iter().take_while(|a| a.parse::<f64>().is_ok()).count();
    let rest = args.len() - nums;
    if (nums == 6 || nums == 7) && rest == 2 {
        StpArgs::Ok(args[nums].clone(), args[nums + 1].clone())
    } else if args.is_empty() {
        StpArgs::Invalid("no frames given".into())
    } else {
        StpArgs::Invalid(format!(
            "{} argument(s); expected 6 or 7 numbers followed by the parent and child frame, or the `--frame-id`/`--child-frame-id` flags",
            args.len()
        ))
    }
}

/// The parent of each child frame, with where the edge comes from.
struct Edge {
    parent: String,
    loc: Loc,
    what: String,
}

impl Agent for TfAgent {
    fn id(&self) -> &'static str {
        "tf"
    }
    fn title(&self) -> &'static str {
        "TF frames"
    }
    fn inputs(&self) -> &'static [crate::workspace::FileKind] {
        use crate::workspace::FileKind as K;
        // The launch graph plus robot descriptions.
        &[
            K::Manifest,
            K::CMakeLists,
            K::SetupPy,
            K::SetupCfg,
            K::LaunchPy,
            K::LaunchXml,
            K::LaunchYaml,
            K::ParamYaml,
            K::Cpp,
            K::Python,
            K::Other,
            K::Urdf,
        ]
    }
    fn relevant(&self, cx: &Context) -> bool {
        cx.graph.instances().any(|(_, _, n)| {
            n.exec.as_deref() == Some(STP) || n.params.iter().any(|p| is_frame_param(&p.name))
        })
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        // URDF edges only when the robot is unambiguous: exactly one fully-evaluated model.
        let robot = match cx.ix.robots.as_slice() {
            [r] if !r.partial => Some(r),
            _ => None,
        };
        let urdf_links: BTreeSet<String> = cx
            .ix
            .robots
            .iter()
            .flat_map(|r| r.links.iter().cloned())
            .collect();
        let urdf_lenient = cx.ix.robots.iter().any(|r| r.partial);
        for root in &cx.graph.roots {
            check_root(cx, root, robot, &urdf_links, urdf_lenient, out);
        }
    }
}

/// F005: follows each frame's parent chain; a chain that comes back to a frame on it is a
/// cycle. Each cycle is reported once, at the edge of its alphabetically first frame.
fn report_cycles(parents: &BTreeMap<String, Edge>, out: &mut Vec<Diagnostic>) {
    let mut reported: BTreeSet<String> = BTreeSet::new();
    for start in parents.keys() {
        let mut path: Vec<&str> = vec![start];
        let mut cur = start.as_str();
        while let Some(e) = parents.get(cur) {
            let p = e.parent.as_str();
            if let Some(i) = path.iter().position(|f| *f == p) {
                let cycle: Vec<&str> = path[i..].to_vec();
                let first = *cycle.iter().min().expect("non-empty");
                if reported.insert(first.to_string()) {
                    let edge = &parents[first];
                    let ring = if cycle.len() == 1 {
                        format!("`{first}` is its own parent")
                    } else {
                        let mut names: Vec<&str> = cycle.iter().rev().copied().collect();
                        names.push(names[0]);
                        names
                            .iter()
                            .map(|n| format!("`{n}`"))
                            .collect::<Vec<_>>()
                            .join(" -> ")
                    };
                    let mut d = Diagnostic::new(
                        "F005",
                        edge.loc,
                        format!(
                            "the transform tree has a cycle: {ring}; tf2 lookups across it fail"
                        ),
                    );
                    for f in cycle.iter().filter(|f| **f != first) {
                        d = d.related(
                            parents[*f].loc,
                            format!(
                                "`{f}` gets parent `{}` from {}",
                                parents[*f].parent, parents[*f].what
                            ),
                        );
                    }
                    out.push(d);
                }
                break;
            }
            path.push(p);
            cur = p;
        }
    }
}

fn check_root(
    cx: &Context,
    root: &RootGraph,
    robot: Option<&crate::facts::RobotModel>,
    urdf_links: &BTreeSet<String>,
    urdf_lenient: bool,
    out: &mut Vec<Diagnostic>,
) {
    let mut parents: BTreeMap<String, Edge> = BTreeMap::new();
    let mut known: BTreeSet<String> = WORLD_FRAMES.iter().map(|s| s.to_string()).collect();
    known.extend(urdf_links.iter().cloned());
    let has_rsp = root
        .nodes
        .iter()
        .any(|n| n.exec.as_deref() == Some("robot_state_publisher"));
    if let (Some(r), true) = (robot, has_rsp) {
        for j in &r.joints {
            if let (Some(p), Some(c)) = (&j.parent, &j.child) {
                parents.insert(
                    c.clone(),
                    Edge {
                        parent: p.clone(),
                        loc: j.loc,
                        what: format!("URDF joint `{}`", j.name),
                    },
                );
            }
        }
    }
    for n in root
        .nodes
        .iter()
        .filter(|n| !n.conditional && n.exec.as_deref() == Some(STP))
    {
        match parse_stp(n) {
            StpArgs::Unknown => {}
            StpArgs::Invalid(why) => out.push(
                Diagnostic::new("F002", n.decl.loc, format!("`static_transform_publisher` cannot use these arguments: {why}; the node exits at startup"))
                    .help("Use `--x 0 --y 0 --z 0 --roll 0 --pitch 0 --yaw 0 --frame-id <parent> --child-frame-id <child>`."),
            ),
            StpArgs::Ok(p, c) => {
                // A leading slash is fine here: tf2 strips it from published transforms.
                for f in [&p, &c] {
                    known.insert(f.trim_start_matches('/').to_string());
                }
                let (p, c) = (p.trim_start_matches('/').to_string(), c.trim_start_matches('/').to_string());
                match parents.get(&c) {
                    Some(e) if e.parent != p => out.push(
                        Diagnostic::new(
                            "F001",
                            n.decl.loc,
                            format!("frame `{c}` gets a second parent: `{p}` here, `{}` from {}; a tf frame can have only one parent", e.parent, e.what),
                        )
                        .related(e.loc, "first parent"),
                    ),
                    Some(_) => {}
                    None => {
                        parents.insert(c, Edge { parent: p, loc: n.decl.loc, what: format!("`{}`", n.display_name()) });
                    }
                }
            }
        }
    }
    report_cycles(&parents, out);
    // Frame parameters of every node.
    for n in &root.nodes {
        // N003 already checks the frame parameters of Nav2 servers.
        let leaf = n
            .name
            .as_deref()
            .or(n.fq_name.as_deref())
            .map(|s| s.rsplit('/').next().unwrap_or(s));
        let nav2_node = leaf.is_some_and(|l| super::nav2::NAV2_NODES.contains(&l));
        for b in n.params.iter().filter(|b| is_frame_param(&b.name)) {
            let Some(ParamValue::Str(v)) = &b.value else {
                continue;
            };
            if v.contains("${") || v.contains("$(") || v.is_empty() {
                continue;
            }
            let base = b.name.rsplit('.').next().unwrap_or(&b.name);
            if v.starts_with('/') {
                if LOOKUP_FRAME_PARAMS.contains(&base) {
                    out.push(slash(v, b.value_loc, value_span(cx.ix, b.value_loc, v)));
                }
                continue;
            }
            if (nav2_node && NAV2_FRAME_PARAMS.contains(&base))
                || urdf_lenient
                || urdf_links.is_empty()
                || known.contains(v.as_str())
                || v.contains('/')
            {
                continue;
            }
            if let Some(c) = closest(v, known.iter().map(|s| s.as_str())) {
                let mut d = Diagnostic::new(
                    "F004",
                    b.value_loc,
                    format!("`{}` = `{v}` is not a known frame; did you mean `{c}`?", b.name),
                )
                .help("tf lookups with an unknown frame fail at runtime. Known frames come from the robot description and static transforms.");
                if let Some(span) = value_span(cx.ix, b.value_loc, v) {
                    d = d.repair(span, v.clone(), known.iter().cloned().collect());
                }
                out.push(d);
            }
        }
    }
}

fn slash(frame: &str, loc: Loc, span: Option<Loc>) -> Diagnostic {
    let fixed = frame.trim_start_matches('/').to_string();
    let mut d = Diagnostic::new("F003", loc, format!("lookup frame `{frame}` starts with `/`; tf2 in ROS 2 rejects such ids in lookups, so they fail"))
        .help(format!("Write `{fixed}`."));
    if let Some(span) = span {
        d = d.fix(Fix {
            description: format!("Use `{fixed}`"),
            safety: FixSafety::Safe,
            edits: vec![TextEdit {
                file: span.file,
                start: span.start,
                end: span.end,
                replacement: fixed,
            }],
        });
    }
    d
}

#[cfg(test)]
mod tests {
    use crate::testutil::analyze_files;

    const PKG: (&str, &str) = (
        "p/package.xml",
        "<package format=\"3\"><name>p</name></package>",
    );
    const URDF: (&str, &str) = (
        "p/urdf/r.urdf",
        "<robot name=\"r\"><link name=\"base_link\"/><link name=\"laser\"/><joint name=\"j\" type=\"fixed\"><parent link=\"base_link\"/><child link=\"laser\"/></joint></robot>",
    );

    #[test]
    fn frame_checks() {
        let launch = "from launch import LaunchDescription\nfrom launch_ros.actions import Node\ndef generate_launch_description():\n    return LaunchDescription([\n        Node(package='robot_state_publisher', executable='robot_state_publisher'),\n        Node(package='tf2_ros', executable='static_transform_publisher', arguments=['--frame-id', 'odom', '--child-frame-id', 'laser']),\n        Node(package='tf2_ros', executable='static_transform_publisher', arguments=['0', '0', '0', 'map', 'odom']),\n        Node(package='tf2_ros', executable='static_transform_publisher', arguments=['--x', '1', '--frame-id', '/map', '--child-frame-id', 'world2']),\n        Node(package='x', executable='y', parameters=[{'frame_id': 'base_lnk', 'odom_frame_id': '/odom'}]),\n    ])\n";
        let d = analyze_files(&[PKG, URDF, ("p/launch/a.launch.py", launch)]);
        let ids: Vec<&str> = d.iter().map(|x| x.rule).collect();
        assert!(
            d.iter()
                .any(|x| x.rule == "F001" && x.message.contains("`laser`")),
            "{d:#?}"
        );
        assert!(ids.contains(&"F002"), "{d:#?}");
        // Only the lookup frame `odom_frame_id`; the static transform's `/map` is fine.
        assert_eq!(ids.iter().filter(|r| **r == "F003").count(), 1, "{d:#?}");
        let f004 = d.iter().find(|x| x.rule == "F004").expect("F004");
        assert!(f004.message.contains("base_link"), "{}", f004.message);
    }

    #[test]
    fn cycles() {
        let launch = "from launch import LaunchDescription\nfrom launch_ros.actions import Node\ndef generate_launch_description():\n    return LaunchDescription([\n        Node(package='tf2_ros', executable='static_transform_publisher', arguments=['--frame-id', 'camera_link', '--child-frame-id', 'camera_rgb_frame']),\n        Node(package='tf2_ros', executable='static_transform_publisher', arguments=['--frame-id', 'camera_rgb_frame', '--child-frame-id', 'camera_optical']),\n        Node(package='tf2_ros', executable='static_transform_publisher', arguments=['--frame-id', 'camera_optical', '--child-frame-id', 'camera_link']),\n        Node(package='tf2_ros', executable='static_transform_publisher', arguments=['--frame-id', 'base_link', '--child-frame-id', 'base_link']),\n    ])\n";
        let d = analyze_files(&[PKG, ("p/launch/a.launch.py", launch)]);
        let f005: Vec<&str> = d
            .iter()
            .filter(|x| x.rule == "F005")
            .map(|x| x.message.as_str())
            .collect();
        assert_eq!(f005.len(), 2, "{f005:#?}");
        assert!(
            f005.iter()
                .any(|m| m.contains("`base_link` is its own parent")),
            "{f005:#?}"
        );
        assert!(
            f005.iter().any(|m| m.contains("`camera_link`")
                && m.contains("`camera_optical`")
                && m.contains("`camera_rgb_frame`")),
            "{f005:#?}"
        );
    }

    #[test]
    fn valid_transforms_are_silent() {
        let launch = "from launch import LaunchDescription\nfrom launch_ros.actions import Node\ndef generate_launch_description():\n    return LaunchDescription([\n        Node(package='robot_state_publisher', executable='robot_state_publisher'),\n        Node(package='tf2_ros', executable='static_transform_publisher', arguments=['0', '0', '0', '0', '0', '0', 'map', 'odom']),\n        Node(package='tf2_ros', executable='static_transform_publisher', arguments=['--frame-id', 'base_link', '--child-frame-id', 'laser']),\n        Node(package='tf2_ros', executable='static_transform_publisher', arguments=['--frame-id', 'a', '--child-frame-id', 'b', '--ros-args', '--log-level', 'warn']),\n        Node(package='x', executable='y', parameters=[{'frame_id': 'laser', 'base_frame': 'base_link'}]),\n        Node(package='tf2_ros', executable='static_transform_publisher', arguments=['--frame-id', '/world', '--child-frame-id', '/base_link']),\n        Node(package='x', executable='cam', parameters=[{'frame_id': '/laser', 'child_frame_id': '/laser2'}]),\n    ])\n";
        let d = analyze_files(&[PKG, URDF, ("p/launch/a.launch.py", launch)]);
        assert!(d.iter().all(|x| !x.rule.starts_with('F')), "{d:#?}");
    }
}
