//! Network / middleware environment agent: what launch files put in the environment of the
//! nodes they start.
//!
//! - D001 `ROS_DOMAIN_ID` is not an integer in 0–232 (participants fail to start; above 232
//!   the discovery ports overflow).
//! - D002 `RMW_IMPLEMENTATION` is a near-typo of a known middleware (the node aborts:
//!   "RMW implementation not installed").
//! - D003 a DDS configuration file (`CYCLONEDDS_URI`, `FASTRTPS_DEFAULT_PROFILES_FILE`,
//!   `FASTDDS_DEFAULT_PROFILES_FILE`) inside the workspace does not exist or is not XML.
//!   The same for a Zenoh configuration file (`ZENOH_SESSION_CONFIG_URI`,
//!   `ZENOH_ROUTER_CONFIG_URI`) that does not exist: rmw_zenoh_cpp fails to initialize.
//! - D004 `ROS_LOCALHOST_ONLY` on Jazzy or newer, where it is deprecated in favour of
//!   `ROS_AUTOMATIC_DISCOVERY_RANGE` (only when the distribution is known).
//! - D005 an rmw_zenoh_cpp variable it cannot use: a `ZENOH_CONFIG_OVERRIDE` pair that is
//!   not `key=JSON5` (ignored), a `ZENOH_ROUTER_CHECK_ATTEMPTS` that is not a number (read
//!   as 0: wait for a router forever), shared-memory sizes that are not positive or not a
//!   power of 2 (ignored).

use super::{Agent, Context};
use crate::diag::Diagnostic;
use crate::resolve::{resolve_path, PathRes};
use crate::util::closest;
use std::collections::HashSet;

pub struct NetworkAgent;

/// Middleware implementations a `RMW_IMPLEMENTATION` value is compared against.
pub const KNOWN_RMW: &[&str] = &[
    "rmw_fastrtps_cpp",
    "rmw_fastrtps_dynamic_cpp",
    "rmw_cyclonedds_cpp",
    "rmw_connextdds",
    "rmw_connext_cpp",
    "rmw_gurumdds_cpp",
    "rmw_zenoh_cpp",
    "rmw_iceoryx_cpp",
    "rmw_ecal_dynamic_cpp",
    "rmw_ecal_proto_cpp",
    "rmw_opendds_cpp",
];

/// Variables that point a middleware at its XML configuration.
pub const DDS_FILE_VARS: &[&str] = &["CYCLONEDDS_URI", "FASTRTPS_DEFAULT_PROFILES_FILE", "FASTDDS_DEFAULT_PROFILES_FILE"];

/// A `ROS_DOMAIN_ID` value the middleware accepts: an integer from 0 to 232.
pub fn domain_id_valid(v: &str) -> bool {
    v.parse::<u32>().is_ok_and(|d| d <= 232)
}

/// The paths named by a DDS configuration variable (comma separated, `file://` allowed).
/// Inline XML (a value starting with `<`) names no file.
pub fn dds_config_paths(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(',')
        .map(|p| p.trim().trim_start_matches("file://"))
        .filter(|p| !p.is_empty() && !p.starts_with('<'))
}

/// Variables that point rmw_zenoh_cpp at a JSON5 configuration file (a plain path).
pub const ZENOH_FILE_VARS: &[&str] = &["ZENOH_SESSION_CONFIG_URI", "ZENOH_ROUTER_CONFIG_URI"];

/// rmw_zenoh_cpp variables whose values it parses (`zenoh_config.cpp`).
pub const ZENOH_VALUE_VARS: &[&str] = &["ZENOH_CONFIG_OVERRIDE", "ZENOH_ROUTER_CHECK_ATTEMPTS", "ZENOH_SHM_ALLOC_SIZE", "ZENOH_SHM_MESSAGE_SIZE_THRESHOLD"];

/// The integer `strtoll` reads from the start of `v`, or None when it reads no digits.
fn leading_int(v: &str) -> Option<i128> {
    let t = v.trim_start();
    let (sign, digits) = match t.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, t.strip_prefix('+').unwrap_or(t)),
    };
    let n: String = digits.chars().take_while(char::is_ascii_digit).take(30).collect();
    n.parse::<i128>().ok().map(|x| sign * x)
}

/// A JSON5 value as `ZENOH_CONFIG_OVERRIDE` needs it: a quoted string, number, boolean,
/// null, array or object. A bare word such as `peer` is not one.
fn json5_value(v: &str) -> bool {
    let v = v.trim();
    let wrapped = |a: char, b: char| v.len() >= 2 && v.starts_with(a) && v.ends_with(b);
    wrapped('"', '"')
        || wrapped('\'', '\'')
        || wrapped('[', ']')
        || wrapped('{', '}')
        || matches!(v, "true" | "false" | "null" | "Infinity" | "-Infinity" | "+Infinity" | "NaN")
        || v.parse::<f64>().is_ok()
        || v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")).is_some_and(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()))
}

/// What rmw_zenoh_cpp does wrong with this value of one of its variables, if anything.
pub fn zenoh_value_problem(name: &str, v: &str) -> Option<String> {
    match name {
        "ZENOH_CONFIG_OVERRIDE" => {
            for pair in v.split(';').filter(|p| !p.trim().is_empty()) {
                match pair.split_once('=') {
                    Some((key, val)) if !val.is_empty() => {
                        if !json5_value(val) {
                            return Some(format!(
                                "the value of `{key}` in `ZENOH_CONFIG_OVERRIDE` is not JSON5 (`{val}`), so rmw_zenoh_cpp ignores this override"
                            ));
                        }
                    }
                    _ => return Some(format!("`{pair}` in `ZENOH_CONFIG_OVERRIDE` is not `key=value`, so rmw_zenoh_cpp skips it without a message")),
                }
            }
            None
        }
        "ZENOH_ROUTER_CHECK_ATTEMPTS" if !v.is_empty() && leading_int(v).is_none() => Some(format!(
            "`ZENOH_ROUTER_CHECK_ATTEMPTS` = `{v}` is not a number; rmw_zenoh_cpp reads it as 0 and waits for a Zenoh router forever"
        )),
        "ZENOH_SHM_ALLOC_SIZE" if !v.is_empty() && leading_int(v).is_none_or(|n| n <= 0) => {
            Some(format!("`ZENOH_SHM_ALLOC_SIZE` = `{v}` is not a positive number, so rmw_zenoh_cpp ignores it"))
        }
        "ZENOH_SHM_MESSAGE_SIZE_THRESHOLD" if !v.is_empty() => match leading_int(v) {
            Some(n) if n > 0 && n & (n - 1) != 0 => Some(format!(
                "`ZENOH_SHM_MESSAGE_SIZE_THRESHOLD` = `{v}` is not a power of 2, so rmw_zenoh_cpp ignores it"
            )),
            Some(n) if n > 0 => None,
            _ => Some(format!("`ZENOH_SHM_MESSAGE_SIZE_THRESHOLD` = `{v}` is not a positive number, so rmw_zenoh_cpp ignores it")),
        },
        _ => None,
    }
}

const VARS: &[&str] = &[
    "ROS_DOMAIN_ID",
    "RMW_IMPLEMENTATION",
    "CYCLONEDDS_URI",
    "FASTRTPS_DEFAULT_PROFILES_FILE",
    "FASTDDS_DEFAULT_PROFILES_FILE",
    "ROS_LOCALHOST_ONLY",
    "ZENOH_SESSION_CONFIG_URI",
    "ZENOH_ROUTER_CONFIG_URI",
    "ZENOH_CONFIG_OVERRIDE",
    "ZENOH_ROUTER_CHECK_ATTEMPTS",
    "ZENOH_SHM_ALLOC_SIZE",
    "ZENOH_SHM_MESSAGE_SIZE_THRESHOLD",
];

impl Agent for NetworkAgent {
    fn id(&self) -> &'static str {
        "network"
    }
    fn title(&self) -> &'static str {
        "Middleware / network environment"
    }
    fn inputs(&self) -> &'static [crate::workspace::FileKind] {
        // The launch graph, plus any loaded file a DDS configuration may point to.
        super::ALL_INPUTS
    }
    fn relevant(&self, cx: &Context) -> bool {
        cx.graph.roots.iter().any(|r| r.env.iter().any(|e| VARS.contains(&e.name.as_str())))
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        let mut seen = HashSet::new();
        let newer = matches!(cx.ix.installed.distro.as_deref(), Some("jazzy" | "kilted" | "rolling"));
        for e in cx.graph.roots.iter().flat_map(|r| r.env.iter()) {
            if !seen.insert(e.loc) || cx.ix.external_files.contains(&e.loc.file) {
                continue;
            }
            let Some(v) = e.value.as_deref().map(str::trim) else { continue };
            if v.contains("$(") || v.contains("@arg:") {
                continue;
            }
            match e.name.as_str() {
                "ROS_DOMAIN_ID" if !v.is_empty() => {
                    if !domain_id_valid(v) {
                        out.push(
                            Diagnostic::new("D001", e.loc, format!("`ROS_DOMAIN_ID` = `{v}` is not a domain ID (an integer from 0 to 232); participants fail to start"))
                                .help("Use 0–101 or 215–232 to stay clear of ephemeral ports on Linux."),
                        );
                    }
                }
                "RMW_IMPLEMENTATION" if !KNOWN_RMW.contains(&v) => {
                    if let Some(c) = closest(v, KNOWN_RMW.iter().copied()) {
                        out.push(
                            Diagnostic::new("D002", e.loc, format!("`RMW_IMPLEMENTATION` = `{v}` is not a middleware; nodes abort at startup (\"RMW implementation not installed\")"))
                                .help(format!("Did you mean `{c}`?")),
                        );
                    }
                }
                name if DDS_FILE_VARS.contains(&name) => {
                    for path in dds_config_paths(v) {
                        match resolve_path(cx.ix, path) {
                            PathRes::Missing(rel) => out.push(Diagnostic::new(
                                "D003",
                                e.loc,
                                format!("`{name}` points to `{rel}`, which does not exist; the middleware ignores it or fails to start"),
                            )),
                            PathRes::Found(f) if roxmltree::Document::parse(&cx.ix.file(f).text).is_err() => out.push(Diagnostic::new(
                                "D003",
                                e.loc,
                                format!("`{name}` points to `{}`, which is not valid XML", cx.ix.file(f).rel_path),
                            )),
                            _ => {}
                        }
                    }
                }
                name if ZENOH_FILE_VARS.contains(&name) && !v.is_empty() => {
                    if let PathRes::Missing(rel) = resolve_path(cx.ix, v.trim_start_matches("file://")) {
                        out.push(Diagnostic::new(
                            "D003",
                            e.loc,
                            format!("`{name}` points to `{rel}`, which does not exist; rmw_zenoh_cpp fails to initialize (\"Invalid configuration file\")"),
                        ));
                    }
                }
                name if ZENOH_VALUE_VARS.contains(&name) => {
                    if let Some(m) = zenoh_value_problem(name, v) {
                        out.push(Diagnostic::new("D005", e.loc, m));
                    }
                }
                "ROS_LOCALHOST_ONLY" if newer => out.push(
                    Diagnostic::new("D004", e.loc, "`ROS_LOCALHOST_ONLY` is deprecated since Jazzy")
                        .help("Use `ROS_AUTOMATIC_DISCOVERY_RANGE=LOCALHOST` (and `ROS_STATIC_PEERS` if needed)."),
                ),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::testutil::analyze_files;

    #[test]
    fn environment_checks() {
        let launch = "from launch import LaunchDescription\nfrom launch.actions import SetEnvironmentVariable\nfrom launch_ros.actions import Node\nimport os\nfrom ament_index_python.packages import get_package_share_directory\ndef generate_launch_description():\n    return LaunchDescription([\n        SetEnvironmentVariable('ROS_DOMAIN_ID', '300'),\n        SetEnvironmentVariable('RMW_IMPLEMENTATION', 'rmw_cyclonedds_cp'),\n        SetEnvironmentVariable('CYCLONEDDS_URI', 'file://' + os.path.join(get_package_share_directory('p'), 'config', 'cyclone.xml')),\n        Node(package='p', executable='x'),\n    ])\n";
        let d = analyze_files(&[
            ("p/package.xml", "<package format=\"3\"><name>p</name></package>"),
            ("p/launch/a.launch.py", launch),
        ]);
        let ids: Vec<&str> = d.iter().map(|x| x.rule).collect();
        for r in ["D001", "D002", "D003"] {
            assert!(ids.contains(&r), "{r}: {d:#?}");
        }
        assert!(d.iter().any(|x| x.rule == "D002" && x.help.as_deref().is_some_and(|h| h.contains("rmw_cyclonedds_cpp"))), "{d:#?}");
    }

    #[test]
    fn zenoh_environment() {
        let launch = "from launch import LaunchDescription\nfrom launch.actions import SetEnvironmentVariable\nfrom launch_ros.actions import Node\nimport os\nfrom ament_index_python.packages import get_package_share_directory\ndef generate_launch_description():\n    return LaunchDescription([\n        SetEnvironmentVariable('RMW_IMPLEMENTATION', 'rmw_zenoh_cpp'),\n        SetEnvironmentVariable('ZENOH_SESSION_CONFIG_URI', os.path.join(get_package_share_directory('p'), 'config', 'zenoh.json5')),\n        SetEnvironmentVariable('ZENOH_CONFIG_OVERRIDE', 'mode=peer'),\n        SetEnvironmentVariable('ZENOH_ROUTER_CHECK_ATTEMPTS', '-1'),\n        Node(package='p', executable='x'),\n    ])\n";
        let d = analyze_files(&[
            ("p/package.xml", "<package format=\"3\"><name>p</name></package>"),
            ("p/launch/a.launch.py", launch),
        ]);
        let msgs: Vec<(&str, &str)> = d.iter().filter(|x| x.rule.starts_with('D')).map(|x| (x.rule, x.message.as_str())).collect();
        assert_eq!(msgs.len(), 2, "{msgs:#?}");
        assert!(msgs.iter().any(|(r, m)| *r == "D003" && m.contains("zenoh.json5")), "{msgs:#?}");
        assert!(msgs.iter().any(|(r, m)| *r == "D005" && m.contains("`peer`")), "{msgs:#?}");
    }

    #[test]
    fn zenoh_values() {
        use super::zenoh_value_problem as p;
        assert!(p("ZENOH_CONFIG_OVERRIDE", "mode=\"peer\";connect/endpoints=[\"tcp/10.0.0.2:7447\"];scouting/multicast/enabled=false").is_none());
        assert!(p("ZENOH_CONFIG_OVERRIDE", "transport/shared_memory/enabled=true;").is_none());
        assert!(p("ZENOH_CONFIG_OVERRIDE", "mode=peer").is_some());
        assert!(p("ZENOH_CONFIG_OVERRIDE", "mode").is_some());
        assert!(p("ZENOH_ROUTER_CHECK_ATTEMPTS", "0").is_none());
        assert!(p("ZENOH_ROUTER_CHECK_ATTEMPTS", "-1").is_none());
        assert!(p("ZENOH_ROUTER_CHECK_ATTEMPTS", "ten").is_some());
        assert!(p("ZENOH_SHM_MESSAGE_SIZE_THRESHOLD", "4096").is_none());
        assert!(p("ZENOH_SHM_MESSAGE_SIZE_THRESHOLD", "1000").is_some());
        assert!(p("ZENOH_SHM_ALLOC_SIZE", "0").is_some());
        assert!(p("ZENOH_SHM_ALLOC_SIZE", "48MB").is_none());
    }

    #[test]
    fn valid_environment_is_silent() {
        let launch = "from launch import LaunchDescription\nfrom launch.actions import SetEnvironmentVariable\nfrom launch_ros.actions import Node\ndef generate_launch_description():\n    return LaunchDescription([\n        SetEnvironmentVariable('ROS_DOMAIN_ID', '42'),\n        SetEnvironmentVariable('RMW_IMPLEMENTATION', 'rmw_my_custom_middleware'),\n        Node(package='p', executable='x'),\n    ])\n";
        let d = analyze_files(&[("p/package.xml", "<package format=\"3\"><name>p</name></package>"), ("p/launch/a.launch.py", launch)]);
        assert!(d.iter().all(|x| !x.rule.starts_with('D')), "{d:#?}");
    }
}
