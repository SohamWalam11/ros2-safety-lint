//! ros_gz_bridge agent: the topics bridged between ROS 2 and Gazebo, written as YAML
//! configuration or as `parameter_bridge` arguments in launch files.
//!
//! - G001 a ROS type and Gazebo type the bridge has no converter for, or a type written in
//!   a form the bridge does not match (`std_msgs/String`, `gz::msgs::Twist`);
//! - G002 a YAML entry the bridge rejects (missing type, conflicting topic keys, unknown
//!   direction or QoS profile);
//! - G003 a `parameter_bridge` argument that does not parse: the program prints its usage
//!   and exits, so nothing in that launch action is bridged;
//! - G004 a YAML key the bridge does not read (ignored without a message);
//! - G005 a one-way `parameter_bridge` argument against the ROS side of the topic: `]` (ROS
//!   to Gazebo) where ROS nodes only subscribe, `[` (Gazebo to ROS) where they only
//!   publish. Checked only when every other node of the launch graph has known endpoints.
//!
//! Type pairs are checked against the ros_gz_bridge mapping table (`mappings.py`). A pair is
//! reported only when both types are in the table and the table does not pair them, so
//! converters added in newer releases or custom builds cannot produce a false report.
//! Values filled in later (`{{ … }}`, `$(…)`, `${…}`) are never checked.

use super::{value_span, Agent, Context};
use crate::diag::Diagnostic;
use crate::extract::yaml::{self, YNode};
use crate::source::{FileId, Loc};
use crate::util::closest;
use crate::workspace::FileKind;
use std::collections::HashSet;

pub struct GzBridgeAgent;

/// (ROS type, Gazebo message) pairs ros_gz_bridge converts: ros_gz_bridge/mappings.py.
const MAPPINGS: &[(&str, &str)] = &[
    ("builtin_interfaces/msg/Time", "Time"),
    ("actuator_msgs/msg/Actuators", "Actuators"),
    ("geometry_msgs/msg/Point", "Vector3d"),
    ("geometry_msgs/msg/Pose", "Pose"),
    ("geometry_msgs/msg/PoseArray", "Pose_V"),
    ("geometry_msgs/msg/PoseStamped", "Pose"),
    ("geometry_msgs/msg/PoseWithCovariance", "PoseWithCovariance"),
    (
        "geometry_msgs/msg/PoseWithCovarianceStamped",
        "PoseWithCovariance",
    ),
    ("geometry_msgs/msg/Quaternion", "Quaternion"),
    ("geometry_msgs/msg/Transform", "Pose"),
    ("geometry_msgs/msg/TransformStamped", "Pose"),
    ("geometry_msgs/msg/Twist", "Twist"),
    ("geometry_msgs/msg/TwistStamped", "Twist"),
    (
        "geometry_msgs/msg/TwistWithCovariance",
        "TwistWithCovariance",
    ),
    (
        "geometry_msgs/msg/TwistWithCovarianceStamped",
        "TwistWithCovariance",
    ),
    ("geometry_msgs/msg/Wrench", "Wrench"),
    ("geometry_msgs/msg/WrenchStamped", "Wrench"),
    ("geometry_msgs/msg/Vector3", "Vector3d"),
    ("gps_msgs/msg/GPSFix", "NavSat"),
    ("marine_acoustic_msgs/msg/Dvl", "DVLVelocityTracking"),
    ("nav_msgs/msg/Odometry", "Odometry"),
    ("nav_msgs/msg/Odometry", "OdometryWithCovariance"),
    ("rcl_interfaces/msg/ParameterValue", "Any"),
    ("ros_gz_interfaces/msg/AirSpeed", "AirSpeed"),
    ("ros_gz_interfaces/msg/Altimeter", "Altimeter"),
    ("ros_gz_interfaces/msg/Contact", "Contact"),
    ("ros_gz_interfaces/msg/Contacts", "Contacts"),
    ("ros_gz_interfaces/msg/Dataframe", "Dataframe"),
    ("ros_gz_interfaces/msg/Entity", "Entity"),
    ("ros_gz_interfaces/msg/EntityFactory", "EntityFactory"),
    ("ros_gz_interfaces/msg/EntityWrench", "EntityWrench"),
    ("ros_gz_interfaces/msg/Float32Array", "Float_V"),
    ("ros_gz_interfaces/msg/GuiCamera", "GUICamera"),
    ("ros_gz_interfaces/msg/JointWrench", "JointWrench"),
    ("ros_gz_interfaces/msg/Light", "Light"),
    (
        "ros_gz_interfaces/msg/LogicalCameraImage",
        "LogicalCameraImage",
    ),
    (
        "ros_gz_interfaces/msg/LogPlaybackStatistics",
        "LogPlaybackStatistics",
    ),
    ("ros_gz_interfaces/msg/MaterialColor", "MaterialColor"),
    ("ros_gz_interfaces/msg/ParamVec", "Param"),
    ("ros_gz_interfaces/msg/ParamVec", "Param_V"),
    ("ros_gz_interfaces/msg/SensorNoise", "SensorNoise"),
    ("ros_gz_interfaces/msg/StringVec", "StringMsg_V"),
    ("ros_gz_interfaces/msg/TrackVisual", "TrackVisual"),
    ("ros_gz_interfaces/msg/VideoRecord", "VideoRecord"),
    ("ros_gz_interfaces/msg/WorldStatistics", "WorldStatistics"),
    ("rosgraph_msgs/msg/Clock", "Clock"),
    ("sensor_msgs/msg/BatteryState", "BatteryState"),
    ("sensor_msgs/msg/CameraInfo", "CameraInfo"),
    ("sensor_msgs/msg/FluidPressure", "FluidPressure"),
    ("sensor_msgs/msg/Image", "Image"),
    ("sensor_msgs/msg/Imu", "IMU"),
    ("sensor_msgs/msg/JointState", "Model"),
    ("sensor_msgs/msg/Joy", "Joy"),
    ("sensor_msgs/msg/LaserScan", "LaserScan"),
    ("sensor_msgs/msg/MagneticField", "Magnetometer"),
    ("sensor_msgs/msg/NavSatFix", "NavSat"),
    ("sensor_msgs/msg/PointCloud2", "PointCloudPacked"),
    ("sensor_msgs/msg/Range", "LaserScan"),
    ("std_msgs/msg/Bool", "Boolean"),
    ("std_msgs/msg/ColorRGBA", "Color"),
    ("std_msgs/msg/Empty", "Empty"),
    ("std_msgs/msg/Float32", "Float"),
    ("std_msgs/msg/Float64", "Double"),
    ("std_msgs/msg/Header", "Header"),
    ("std_msgs/msg/Int32", "Int32"),
    ("std_msgs/msg/UInt32", "UInt32"),
    ("std_msgs/msg/String", "StringMsg"),
    ("tf2_msgs/msg/TFMessage", "Pose_V"),
    ("trajectory_msgs/msg/JointTrajectory", "JointTrajectory"),
    (
        "vision_msgs/msg/Detection2DArray",
        "AnnotatedAxisAligned2DBox_V",
    ),
    ("vision_msgs/msg/Detection2D", "AnnotatedAxisAligned2DBox"),
    (
        "vision_msgs/msg/Detection3DArray",
        "AnnotatedOriented3DBox_V",
    ),
    ("vision_msgs/msg/Detection3D", "AnnotatedOriented3DBox"),
];

/// Keys the bridge reads from a YAML entry (`ign_*` from older releases are accepted too).
const KEYS: &[&str] = &[
    "topic_name",
    "ros_topic_name",
    "gz_topic_name",
    "ros_type_name",
    "gz_type_name",
    "direction",
    "publisher_queue",
    "subscriber_queue",
    "qos_profile",
    "lazy",
    "service_name",
    "gz_req_type_name",
    "gz_rep_type_name",
    "frame_id",
    "ign_topic_name",
    "ign_type_name",
];
const DIRECTIONS: &[&str] = &["BIDIRECTIONAL", "GZ_TO_ROS", "ROS_TO_GZ"];
const QOS_PROFILES: &[&str] = &[
    "CLOCK",
    "SENSOR_DATA",
    "PARAMETERS",
    "SERVICES",
    "PARAMETER_EVENTS",
    "ROSOUT",
    "SYSTEM_DEFAULT",
    "BEST_AVAILABLE",
];
const BRIDGE_PACKAGES: &[&str] = &["ros_gz_bridge", "ros_ign_bridge"];

const INPUTS: &[FileKind] = &[
    FileKind::GzBridge,
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

/// A YAML file whose top level is a sequence of bridge entries.
pub fn is_bridge_config(text: &str) -> bool {
    if !(text.contains("ros_type_name")
        || text.contains("gz_type_name")
        || text.contains("ign_type_name"))
    {
        return false;
    }
    matches!(
        yaml::parse(text).ok().as_deref(),
        Some([YNode::Seq { .. }, ..])
    )
}

impl Agent for GzBridgeAgent {
    fn id(&self) -> &'static str {
        "gzbridge"
    }
    fn title(&self) -> &'static str {
        "ros_gz_bridge topics (YAML configuration and parameter_bridge arguments)"
    }
    fn inputs(&self) -> &'static [FileKind] {
        INPUTS
    }
    fn relevant(&self, cx: &Context) -> bool {
        cx.ix.kinds.contains(&FileKind::GzBridge)
            || cx.graph.instances().any(|(_, _, n)| is_bridge_node(n))
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        for f in (0..cx.ix.kinds.len())
            .filter(|i| cx.ix.kinds[*i] == FileKind::GzBridge && !cx.ix.external_files.contains(i))
        {
            let Ok(docs) = yaml::parse(&cx.ix.file(f).text) else {
                continue;
            };
            for entry in docs.first().map(YNode::items).unwrap_or_default() {
                check_entry(f, entry, out);
            }
        }
        let mut seen = HashSet::new();
        for (r, _, n) in cx.graph.instances() {
            if !is_bridge_node(n) || cx.ix.external_files.contains(&n.decl.loc.file) {
                continue;
            }
            for arg in n.arguments.iter().flatten() {
                if arg.contains('@')
                    && !templated(arg)
                    && seen.insert((n.decl.loc.file, n.decl.loc.start, arg.clone()))
                {
                    check_argument(cx, n.decl.loc, arg, out);
                    check_direction(cx, r, n, arg, out);
                }
            }
        }
    }
}

/// G005: a one-way bridge pointing against the ROS side of the topic, in a launch graph
/// whose every other node's endpoints are known.
fn check_direction(
    cx: &Context,
    root: usize,
    bridge: &crate::resolve::NodeInstance,
    arg: &str,
    out: &mut Vec<Diagnostic>,
) {
    use crate::facts::EndpointKind;
    let Ok(s) = parse_argument(arg) else { return };
    let dir = arg[s.ros_off + s.ros.len()..].chars().next();
    if s.service || s.no_direction || !matches!(dir, Some('[' | ']')) {
        return;
    }
    let g = &cx.graph.roots[root];
    if g.partial {
        return;
    }
    let mut topic = crate::resolve::expand_name(
        &arg[..s.ros_off - 1],
        &bridge.namespace,
        bridge.name.as_deref().unwrap_or("ros_gz_bridge"),
    );
    if let Some((_, to, _)) = bridge.remaps.iter().find(|(from, _, _)| *from == topic) {
        topic = to.clone();
    }
    let (mut pubs, mut subs) = (false, false);
    for n in g.nodes.iter().filter(|n| !is_bridge_node(n)) {
        let (eps, complete) = crate::resolve::instance_endpoints(cx.ix, n);
        if !complete {
            return; // a node whose topics are not all known could be the missing side
        }
        for e in eps.iter().filter(|e| e.name == topic) {
            pubs |= e.kind == EndpointKind::Publisher;
            subs |= e.kind == EndpointKind::Subscription;
        }
    }
    let (message, help) = match dir {
        Some(']') if subs && !pubs => (
            format!("ROS nodes only subscribe to `{topic}`, but `]` bridges it from ROS to Gazebo only, so they never receive Gazebo's messages"),
            "Use `[` to bridge from Gazebo to ROS (or `@` for both directions).",
        ),
        Some('[') if pubs && !subs => (
            format!("ROS nodes only publish `{topic}`, but `[` bridges it from Gazebo to ROS only, so Gazebo never receives these messages"),
            "Use `]` to bridge from ROS to Gazebo (or `@` for both directions).",
        ),
        _ => return,
    };
    let loc = value_span(cx.ix, bridge.decl.loc, arg)
        .filter(|l| cx.ix.file(l.file).slice(*l) == arg)
        .unwrap_or(bridge.decl.loc);
    out.push(Diagnostic::new("G005", loc, message).help(help));
}

fn is_bridge_node(n: &crate::resolve::NodeInstance) -> bool {
    n.pkg
        .as_deref()
        .is_some_and(|p| BRIDGE_PACKAGES.contains(&p))
        && n.exec.as_deref() == Some("parameter_bridge")
}

/// A value filled in later (Jinja `{{ … }}`, launch `$(…)`, xacro `${…}`).
fn templated(s: &str) -> bool {
    s.contains("{{") || s.contains("$(") || s.contains("${")
}

/// The allowed word meant by `typed`: itself, the only word it is a prefix of
/// (`SENSOR` for `SENSOR_DATA`), or the closest spelling.
fn guess(typed: &str, allowed: &[&'static str]) -> Option<&'static str> {
    if let Some(a) = allowed.iter().find(|a| **a == typed) {
        return Some(a);
    }
    let prefixed: Vec<&'static str> = allowed
        .iter()
        .copied()
        .filter(|a| a.starts_with(typed))
        .collect();
    if prefixed.len() == 1 && typed.len() >= 3 {
        return Some(prefixed[0]);
    }
    closest(typed, allowed.iter().copied())
}

/// The span of a scalar's text, without quotes.
fn inner(n: &YNode) -> (usize, usize) {
    match n {
        YNode::Scalar {
            quoted: true,
            start,
            end,
            ..
        } if end - start >= 2 => (start + 1, end - 1),
        _ => (n.start(), n.end()),
    }
}

/// A problem with a ROS / Gazebo type pair: message, help, and the side to rewrite.
struct TypeIssue {
    message: String,
    help: Option<String>,
    /// (true for the ROS type, replacement text)
    fix: Option<(bool, String)>,
}

fn gz_name(gz: &str) -> Option<&str> {
    gz.strip_prefix("gz.msgs.")
        .or_else(|| gz.strip_prefix("ignition.msgs."))
}

fn pair_ok(ros: &str, gz: &str) -> bool {
    MAPPINGS.iter().any(|(r, g)| *r == ros && *g == gz)
}

fn check_types(ros: &str, gz: &str) -> Option<TypeIssue> {
    if templated(ros) || templated(gz) || ros.contains("/srv/") || ros.is_empty() || gz.is_empty() {
        return None;
    }
    let ros_ok = {
        let parts: Vec<&str> = ros.split('/').collect();
        parts.len() == 3 && parts[1] == "msg"
    };
    match (ros_ok, gz_name(gz)) {
        (true, Some(name)) => {
            let partners: Vec<String> = MAPPINGS
                .iter()
                .filter(|(r, _)| *r == ros)
                .map(|(_, g)| format!("gz.msgs.{g}"))
                .collect();
            let gz_known = MAPPINGS.iter().any(|(_, g)| *g == name);
            if pair_ok(ros, name) || partners.is_empty() || !gz_known {
                return None;
            }
            Some(TypeIssue {
                message: format!("ros_gz_bridge has no converter between `{ros}` and `{gz}`, so this topic is not bridged (\"No template specialization for the pair\")"),
                help: Some(format!("`{ros}` converts to {}.", partners.join(", "))),
                fix: None,
            })
        }
        (false, Some(name)) => {
            let (pkg, msg) = ros.split_once('/')?;
            if msg.contains('/') {
                return None;
            }
            let full = format!("{pkg}/msg/{msg}");
            if !MAPPINGS.iter().any(|(r, _)| *r == full) {
                return None;
            }
            Some(TypeIssue {
                message: format!("ros_gz_bridge matches the exact type string `{full}`; `{ros}` matches no converter, so this topic is not bridged"),
                help: None,
                fix: pair_ok(&full, name).then(|| (true, full.clone())),
            })
        }
        (true, None) => {
            if !(gz.starts_with("gz") || gz.starts_with("ign")) {
                return None;
            }
            let name = gz.rsplit(|c| c == '.' || c == ':').next()?;
            if !pair_ok(ros, name) {
                return None;
            }
            let fixed = format!("gz.msgs.{name}");
            Some(TypeIssue {
                message: format!("`{gz}` is not a Gazebo type name the bridge matches; write `{fixed}`. As written, this topic is not bridged"),
                help: None,
                fix: Some((false, fixed)),
            })
        }
        (false, None) => None,
    }
}

fn type_diag(issue: TypeIssue, loc: Loc, fix_span: Option<(FileId, usize, usize)>) -> Diagnostic {
    let mut d = Diagnostic::new("G001", loc, issue.message);
    if let Some(h) = issue.help {
        d = d.help(h);
    }
    if let (Some((_, repl)), Some((f, s, e))) = (issue.fix, fix_span) {
        d = d.safe_fix(format!("Write `{repl}`"), f, s, e, repl);
    }
    d
}

/// G001, G002 and G004 for one YAML entry.
fn check_entry(f: FileId, e: &YNode, out: &mut Vec<Diagnostic>) {
    if !e.is_map() {
        return;
    }
    let text = |k: &str| e.get(k).and_then(YNode::as_str).filter(|s| !s.is_empty());
    for (k, _) in e.entries() {
        let Some(key) = k.as_str() else { continue };
        if KEYS.contains(&key) || templated(key) {
            continue;
        }
        let mut d = Diagnostic::new(
            "G004",
            Loc::new(f, k.start(), k.end()),
            format!("ros_gz_bridge does not read `{key}`; the key is ignored without a message"),
        );
        if let Some(c) = guess(key, KEYS) {
            let (s, en) = inner(k);
            d = d.help(format!("Did you mean `{c}`?")).safe_fix(
                format!("Rename the key to `{c}`"),
                f,
                s,
                en,
                c,
            );
        }
        out.push(d);
    }
    let first = e
        .entries()
        .first()
        .map_or(Loc::new(f, e.start(), e.start()), |(k, _)| {
            Loc::new(f, k.start(), k.end())
        });
    let topic = match text("ros_topic_name")
        .or_else(|| text("topic_name"))
        .or_else(|| text("gz_topic_name"))
    {
        Some(t) => format!("the entry for `{t}`"),
        None => "this entry".to_string(),
    };
    let mut invalid = |loc: Loc, why: String| {
        out.push(Diagnostic::new(
            "G002",
            loc,
            format!("the bridge skips {topic}: {why}"),
        ));
    };
    for other in ["ros_topic_name", "gz_topic_name"] {
        if let (Some(_), Some((k, _))) = (
            text("topic_name"),
            e.entry(other)
                .filter(|(_, v)| v.as_str().is_some_and(|s| !s.is_empty())),
        ) {
            invalid(
                Loc::new(f, k.start(), k.end()),
                format!("`topic_name` and `{other}` cannot both be set"),
            );
        }
    }
    let legacy = e.get("ign_type_name").is_some() || e.get("ign_topic_name").is_some();
    if text("service_name").is_some() {
        for k in ["gz_req_type_name", "gz_rep_type_name"] {
            if text(k).is_none() {
                invalid(first, format!("a service entry needs `gz_req_type_name` and `gz_rep_type_name`; `{k}` is missing"));
            }
        }
    } else if !legacy {
        for k in ["ros_type_name", "gz_type_name"] {
            if text(k).is_none() {
                invalid(
                    first,
                    format!(
                        "`{k}` is missing; both `ros_type_name` and `gz_type_name` must be set"
                    ),
                );
            }
        }
    }
    for (key, allowed) in [("direction", DIRECTIONS), ("qos_profile", QOS_PROFILES)] {
        let Some(v) = e.get(key) else { continue };
        let Some(value) = v.as_str() else { continue };
        if allowed.contains(&value)
            || templated(value)
            || (key == "qos_profile" && value.is_empty())
        {
            continue;
        }
        let mut d = Diagnostic::new(
            "G002",
            Loc::new(f, v.start(), v.end()),
            format!(
                "the bridge skips {topic}: `{value}` is not a valid {key} ({})",
                allowed.join(", ")
            ),
        );
        let upper = value.trim().to_ascii_uppercase().replace('-', "_");
        if let Some(c) = guess(&upper, allowed) {
            let (s, en) = inner(v);
            d = d.help(format!("Did you mean `{c}`?")).safe_fix(
                format!("Write `{c}`"),
                f,
                s,
                en,
                c,
            );
        }
        out.push(d);
    }
    if let (Some(ros), Some(gz)) = (e.get("ros_type_name"), e.get("gz_type_name")) {
        if let (Some(r), Some(g)) = (ros.as_str(), gz.as_str()) {
            if let Some(issue) = check_types(r, g) {
                let target = match &issue.fix {
                    Some((true, _)) => ros,
                    _ => gz,
                };
                let (s, en) = inner(target);
                let loc = Loc::new(f, target.start(), target.end());
                out.push(type_diag(issue, loc, Some((f, s, en))));
            }
        }
    }
}

/// A parsed `parameter_bridge` argument; offsets are byte positions within the argument.
struct Spec<'a> {
    ros: &'a str,
    ros_off: usize,
    gz: &'a str,
    gz_off: usize,
    /// No `@`, `[` or `]` after the ROS type.
    no_direction: bool,
    service: bool,
}

/// The argument parsing of `parameter_bridge.cpp`; Err is the case where it prints its usage
/// text and exits.
fn parse_argument(a: &str) -> Result<Spec<'_>, &'static str> {
    let d = a
        .find('@')
        .filter(|d| *d > 0)
        .ok_or("no topic name before the first `@`")?;
    let rest_off = d + 1;
    let rest = &a[rest_off..];
    let p = match rest.find('@').filter(|p| *p > 0) {
        Some(p) => Some(p),
        None => match rest.find('[').filter(|p| *p > 0) {
            Some(p) => Some(p),
            None => match rest.find(']') {
                Some(0) => return Err("no ROS type before `]`"),
                other => other,
            },
        },
    };
    let direction = p.map(|p| &rest[p..p + 1]);
    let ros = p.map_or(rest, |p| &rest[..p]);
    // Without a separator the C++ code erases nothing (npos + 1 wraps to 0).
    let gz_off = rest_off + p.map_or(0, |p| p + 1);
    let after = &a[gz_off..];
    if ros.contains("/srv/") {
        return match direction {
            Some("[") | Some("]") => Err("a service bridge cannot have a direction"),
            Some(_) if after.find('@').filter(|q| *q > 0).is_none() => {
                Err("a service needs `@GZ_REQUEST_TYPE@GZ_REPLY_TYPE` after its ROS type")
            }
            _ => Ok(Spec {
                ros,
                ros_off: rest_off,
                gz: "",
                gz_off,
                no_direction: false,
                service: true,
            }),
        };
    }
    if after.is_empty() {
        return Err("no Gazebo type after the separator");
    }
    if after.contains('@') {
        return Err("too many `@` separators");
    }
    Ok(Spec {
        ros,
        ros_off: rest_off,
        gz: after,
        gz_off,
        no_direction: p.is_none(),
        service: false,
    })
}

/// G001 and G003 for one `parameter_bridge` argument.
fn check_argument(cx: &Context, decl: Loc, arg: &str, out: &mut Vec<Diagnostic>) {
    // The argument's own text, when the launch file writes it as one literal.
    let span = value_span(cx.ix, decl, arg).filter(|l| cx.ix.file(l.file).slice(*l) == arg);
    let loc = span.unwrap_or(decl);
    match parse_argument(arg) {
        Err(why) => out.push(
            Diagnostic::new(
                "G003",
                loc,
                format!("parameter_bridge cannot parse `{arg}` ({why}); it prints its usage text and exits, so no topic of this bridge is bridged"),
            )
            .help("Write `TOPIC@ROS_TYPE@GZ_TYPE`, with `[` (Gazebo to ROS) or `]` (ROS to Gazebo) in place of the second `@` for one direction."),
        ),
        Ok(s) if s.no_direction && !s.service => out.push(
            Diagnostic::new(
                "G003",
                loc,
                format!("`{arg}` has no `@`, `[` or `]` before a Gazebo type, so the bridge uses `{}` as both types and this topic is not bridged", s.ros),
            )
            .help("Write `TOPIC@ROS_TYPE@GZ_TYPE`, with `[` (Gazebo to ROS) or `]` (ROS to Gazebo) in place of the second `@` for one direction."),
        ),
        Ok(s) if !s.service => {
            if let Some(issue) = check_types(s.ros, s.gz) {
                let fix_span = span.map(|l| match &issue.fix {
                    Some((true, _)) => (l.file, l.start + s.ros_off, l.start + s.ros_off + s.ros.len()),
                    _ => (l.file, l.start + s.gz_off, l.start + s.gz_off + s.gz.len()),
                });
                out.push(type_diag(issue, loc, fix_span));
            }
        }
        Ok(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::analyze_files;

    const PKG: (&str, &str) = (
        "sim/package.xml",
        "<package format=\"3\"><name>sim</name></package>",
    );

    fn launch(args: &str) -> Vec<Diagnostic> {
        let py = format!(
            "from launch import LaunchDescription\nfrom launch_ros.actions import Node\ndef generate_launch_description():\n    return LaunchDescription([\n        Node(package='ros_gz_bridge', executable='parameter_bridge', arguments=[{args}]),\n    ])\n"
        );
        analyze_files(&[PKG, ("sim/launch/bridge.launch.py", &py)])
    }

    #[test]
    fn argument_parsing_matches_parameter_bridge() {
        assert!(parse_argument("/clock@rosgraph_msgs/msg/Clock[gz.msgs.Clock").is_ok());
        assert!(parse_argument("/cmd@geometry_msgs/msg/Twist]gz.msgs.Twist").is_ok());
        assert!(parse_argument(
            "/s@ros_gz_interfaces/srv/ControlWorld@gz.msgs.WorldControl@gz.msgs.Boolean"
        )
        .is_ok_and(|s| s.service));
        assert!(parse_argument("/s@ros_gz_interfaces/srv/ControlWorld").is_ok_and(|s| s.service));
        assert!(parse_argument("/bat@sensor_msgs/msg/BatteryState@").is_err());
        assert!(parse_argument("@std_msgs/msg/String@gz.msgs.StringMsg").is_err());
        assert!(parse_argument("/a@std_msgs/msg/String@gz.msgs.StringMsg@x").is_err());
        assert!(parse_argument("/a@]gz.msgs.StringMsg").is_err());
        assert!(parse_argument("/clock@rosgraph_msgs/msg/Clock").is_ok_and(|s| s.no_direction));
    }

    #[test]
    fn launch_arguments() {
        let d = launch("'/scan@sensor_msgs/msg/LaserScan[gz.msgs.PointCloudPacked', '/bat@sensor_msgs/msg/BatteryState@', '/chatter@std_msgs/String@gz.msgs.StringMsg', '/cmd_vel@geometry_msgs/msg/Twist]gz::msgs::Twist', '/ok@std_msgs/msg/String@ignition.msgs.StringMsg'");
        let g: Vec<&Diagnostic> = d.iter().filter(|x| x.rule.starts_with('G')).collect();
        assert_eq!(g.len(), 4, "{g:#?}");
        let by = |r: &str, s: &str| {
            g.iter()
                .find(|x| x.rule == r && x.message.contains(s))
                .copied()
                .unwrap_or_else(|| panic!("{r} {s}: {g:#?}"))
        };
        assert!(by("G001", "PointCloudPacked")
            .help
            .as_deref()
            .is_some_and(|h| h.contains("gz.msgs.LaserScan")));
        by("G003", "BatteryState");
        assert!(by("G001", "std_msgs/String").fix.is_some());
        assert!(by("G001", "gz::msgs::Twist").fix.is_some());
    }

    #[test]
    fn yaml_configuration() {
        let yaml = "- ros_topic_name: \"/clock\"\n  gz_topic_name: \"/clock\"\n  ros_type_name: \"rosgraph_msgs/msg/Clock\"\n  gz_type_name: \"gz.msgs.Clock\"\n  direction: gz_to_ros\n\
- topic_name: /scan\n  ros_topic_name: /scan2\n  ros_type_name: sensor_msgs/msg/LaserScan\n  gz_type_name: gz.msgs.LaserScan\n\
- ros_topic_name: /imu\n  ros_type_name: sensor_msgs/msg/Imu\n  gz_type_name: gz.msgs.Image\n  qos_profile: SENSOR\n\
- ros_topic: /odom\n  ros_type_name: nav_msgs/msg/Odometry\n\
- ros_topic_name: \"{{ ns }}/cmd\"\n  ros_type_name: geometry_msgs/msg/Twist\n  gz_type_name: gz.msgs.Twist\n  direction: ROS_TO_GZ\n";
        let d = analyze_files(&[PKG, ("sim/config/bridge.yaml", yaml)]);
        let g: Vec<&Diagnostic> = d.iter().filter(|x| x.rule.starts_with('G')).collect();
        let by = |r: &str, s: &str| {
            g.iter()
                .find(|x| x.rule == r && x.message.contains(s))
                .copied()
                .unwrap_or_else(|| panic!("{r} {s}: {g:#?}"))
        };
        assert!(by("G002", "`gz_to_ros`").fix.is_some());
        by("G002", "`topic_name` and `ros_topic_name`");
        by("G001", "gz.msgs.Image");
        assert!(
            by("G002", "`SENSOR`")
                .fix
                .as_ref()
                .is_some_and(|f| f.edits[0].replacement == "SENSOR_DATA"),
            "{g:#?}"
        );
        assert!(by("G004", "`ros_topic`").fix.is_some());
        by("G002", "`gz_type_name` is missing");
        assert_eq!(g.len(), 6, "{g:#?}");
    }

    #[test]
    fn direction_against_the_ros_side() {
        let node = "#include <rclcpp/rclcpp.hpp>\n#include <sensor_msgs/msg/laser_scan.hpp>\nclass Follower : public rclcpp::Node {\npublic:\n  Follower() : Node(\"follower\") {\n    sub_ = create_subscription<sensor_msgs::msg::LaserScan>(\"/scan\", 10, [](sensor_msgs::msg::LaserScan::SharedPtr) {});\n  }\n  rclcpp::Subscription<sensor_msgs::msg::LaserScan>::SharedPtr sub_;\n};\n";
        let files = |dir: &str| {
            let py = format!("from launch import LaunchDescription\nfrom launch_ros.actions import Node\ndef generate_launch_description():\n    return LaunchDescription([\n        Node(package='ros_gz_bridge', executable='parameter_bridge', arguments=['/scan@sensor_msgs/msg/LaserScan{dir}gz.msgs.LaserScan']),\n        Node(package='sim', executable='follower'),\n    ])\n");
            analyze_files(&[
                ("sim/package.xml", "<package format=\"3\"><name>sim</name><depend>rclcpp</depend><depend>sensor_msgs</depend></package>"),
                ("sim/CMakeLists.txt", "project(sim)\nfind_package(rclcpp REQUIRED)\nadd_executable(follower src/follower.cpp)\ninstall(TARGETS follower DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch DESTINATION share/${PROJECT_NAME})\nament_package()\n"),
                ("sim/src/follower.cpp", node),
                ("sim/launch/bridge.launch.py", &py),
            ])
        };
        let d = files("]");
        assert!(
            d.iter()
                .any(|x| x.rule == "G005" && x.message.contains("only subscribe to `/scan`")),
            "{d:#?}"
        );
        let d = files("[");
        assert!(!d.iter().any(|x| x.rule == "G005"), "{d:#?}");
    }

    #[test]
    fn valid_bridges_are_silent() {
        let d = launch("'/clock@rosgraph_msgs/msg/Clock[gz.msgs.Clock', '/tf@tf2_msgs/msg/TFMessage[ignition.msgs.Pose_V', '/x@my_msgs/msg/Custom@gz.msgs.Custom', '/s@ros_gz_interfaces/srv/SpawnEntity'");
        assert!(!d.iter().any(|x| x.rule.starts_with('G')), "{d:#?}");
        let yaml = "- ros_topic_name: /odom\n  ros_type_name: nav_msgs/msg/Odometry\n  gz_type_name: gz.msgs.OdometryWithCovariance\n  direction: GZ_TO_ROS\n  lazy: true\n- service_name: /spawn\n  ros_type_name: ros_gz_interfaces/srv/SpawnEntity\n  gz_req_type_name: gz.msgs.EntityFactory\n  gz_rep_type_name: gz.msgs.Boolean\n";
        let d = analyze_files(&[PKG, ("sim/config/bridge.yaml", yaml)]);
        assert!(!d.iter().any(|x| x.rule.starts_with('G')), "{d:#?}");
    }
}
