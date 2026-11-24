//! `rosfix doctor`: checks the ROS 2 environment of the current shell.
//!
//! The workspace checks see what launch files put into a node's environment. Most
//! middleware problems come from the shell instead: a stale `setup.bash`, a mistyped
//! `RMW_IMPLEMENTATION`, a DDS profile that only allows shared memory. Those depend on the
//! machine, so they are reported here and never affect `rosfix check`.

use crate::agents::network::{
    dds_config_paths, domain_id_valid, zenoh_value_problem, KNOWN_RMW, ZENOH_FILE_VARS,
    ZENOH_VALUE_VARS,
};
use crate::util::closest;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Note,
    Warning,
    Error,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Note => "note",
            Level::Warning => "warning",
            Level::Error => "error",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Check {
    pub level: Level,
    /// The variable (or topic) the check is about.
    pub subject: String,
    pub message: String,
    pub help: Option<String>,
}

fn check(level: Level, subject: &str, message: impl Into<String>) -> Check {
    Check {
        level,
        subject: subject.to_string(),
        message: message.into(),
        help: None,
    }
}

impl Check {
    fn help(mut self, h: impl Into<String>) -> Self {
        self.help = Some(h.into());
        self
    }
}

const PACK_DISTROS: &[&str] = &["humble", "jazzy", "kilted", "rolling"];
const DISCOVERY_RANGES: &[&str] = &["SUBNET", "LOCALHOST", "OFF", "SYSTEM_DEFAULT"];

/// Where rmw_zenoh_cpp's default session configuration looks for the Zenoh router.
const ZENOH_ROUTER_ADDR: &str = "127.0.0.1:7447";

/// True if something accepts TCP connections at `addr` (within a short timeout).
fn tcp_open(addr: &str) -> bool {
    addr.parse::<std::net::SocketAddr>().is_ok_and(|a| {
        std::net::TcpStream::connect_timeout(&a, std::time::Duration::from_millis(300)).is_ok()
    })
}

/// Runs every check against an environment (variable name → value).
pub fn run(env: &BTreeMap<String, String>) -> Vec<Check> {
    run_with(env, &tcp_open)
}

/// `run`, with the probe that tells whether a TCP port is open (replaced in tests).
fn run_with(env: &BTreeMap<String, String>, port_open: &dyn Fn(&str) -> bool) -> Vec<Check> {
    let get = |k: &str| env.get(k).map(|v| v.trim()).filter(|v| !v.is_empty());
    let mut out = Vec::new();

    // ---- distribution
    let distro = get("ROS_DISTRO");
    match distro {
        None => out.push(
            check(
                Level::Warning,
                "ROS_DISTRO",
                "not set: no ROS 2 environment is sourced in this shell",
            )
            .help("Source /opt/ros/<distro>/setup.bash (and your workspace's install/setup.bash)."),
        ),
        Some(d) if PACK_DISTROS.contains(&d) => {
            out.push(check(Level::Ok, "ROS_DISTRO", d));
        }
        Some(d) => out.push(check(
            Level::Note,
            "ROS_DISTRO",
            format!("`{d}`: rosfix has no knowledge pack for it (packs: humble, jazzy, kilted)"),
        )),
    }
    let newer = matches!(distro, Some("jazzy" | "kilted" | "rolling"));

    // ---- install prefixes
    let prefixes: Vec<&str> = get("AMENT_PREFIX_PATH")
        .map(|v| {
            v.split(if cfg!(windows) { ';' } else { ':' })
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if prefixes.is_empty() {
        out.push(check(
            Level::Warning,
            "AMENT_PREFIX_PATH",
            "not set: installed packages cannot be found",
        ));
    } else {
        let missing: Vec<&str> = prefixes
            .iter()
            .copied()
            .filter(|p| !Path::new(p).is_dir())
            .collect();
        for p in &missing {
            out.push(
                check(Level::Warning, "AMENT_PREFIX_PATH", format!("`{p}` does not exist"))
                    .help("A workspace was deleted or moved after it was sourced; open a new shell and source again."),
            );
        }
        if missing.is_empty() {
            out.push(check(
                Level::Ok,
                "AMENT_PREFIX_PATH",
                format!("{} prefix(es)", prefixes.len()),
            ));
        }
    }
    let installed = |pkg: &str| {
        prefixes.iter().any(|p| {
            Path::new(p)
                .join("share/ament_index/resource_index/packages")
                .join(pkg)
                .exists()
        })
    };

    // ---- middleware
    let rmw = get("RMW_IMPLEMENTATION");
    match rmw {
        None => out.push(check(
            Level::Ok,
            "RMW_IMPLEMENTATION",
            "not set (the distribution's default middleware)",
        )),
        Some(v) if KNOWN_RMW.contains(&v) => {
            if !prefixes.is_empty() && !installed(v) {
                out.push(
                    check(
                        Level::Error,
                        "RMW_IMPLEMENTATION",
                        format!("`{v}` is not installed in any prefix; nodes abort at startup"),
                    )
                    .help(format!(
                        "Install it (e.g. ros-<distro>-{}) or unset the variable.",
                        v.replace('_', "-")
                    )),
                );
            } else {
                out.push(check(Level::Ok, "RMW_IMPLEMENTATION", v));
            }
        }
        Some(v) => match closest(v, KNOWN_RMW.iter().copied()) {
            Some(c) => out.push(
                check(
                    Level::Error,
                    "RMW_IMPLEMENTATION",
                    format!("`{v}` is not a middleware; nodes abort at startup"),
                )
                .help(format!("Did you mean `{c}`?")),
            ),
            None => out.push(check(
                Level::Note,
                "RMW_IMPLEMENTATION",
                format!("`{v}` is not a middleware rosfix knows"),
            )),
        },
    }
    // Which middleware actually runs: the variable, else Fast DDS (the default of every
    // distribution rosfix knows).
    let effective = match (rmw, distro) {
        (Some(v), _) => Some(v),
        (None, Some(d)) if PACK_DISTROS.contains(&d) => Some("rmw_fastrtps_cpp"),
        _ => None,
    };

    // ---- domain
    if let Some(v) = get("ROS_DOMAIN_ID") {
        if !domain_id_valid(v) {
            out.push(check(
                Level::Error,
                "ROS_DOMAIN_ID",
                format!("`{v}` is not a domain ID (an integer from 0 to 232); participants fail to start"),
            ));
        } else if let Ok(102..=214) = v.parse::<u32>() {
            out.push(
                check(
                    Level::Note,
                    "ROS_DOMAIN_ID",
                    format!("`{v}` can collide with ephemeral ports on Linux"),
                )
                .help("Use 0-101 or 215-232."),
            );
        } else {
            out.push(check(Level::Ok, "ROS_DOMAIN_ID", v));
        }
    }

    // ---- discovery
    let range = get("ROS_AUTOMATIC_DISCOVERY_RANGE");
    if let Some(v) = get("ROS_LOCALHOST_ONLY") {
        if v != "0" && v != "1" {
            out.push(check(
                Level::Warning,
                "ROS_LOCALHOST_ONLY",
                format!("`{v}` is neither 0 nor 1"),
            ));
        }
        if newer {
            let c = check(
                Level::Warning,
                "ROS_LOCALHOST_ONLY",
                "deprecated since Jazzy",
            );
            out.push(if range.is_some() {
                c.help("ROS_AUTOMATIC_DISCOVERY_RANGE is also set and the two can disagree; unset ROS_LOCALHOST_ONLY.")
            } else {
                c.help("Use ROS_AUTOMATIC_DISCOVERY_RANGE=LOCALHOST instead.")
            });
        }
    }
    if let Some(v) = range {
        if !DISCOVERY_RANGES.contains(&v) {
            out.push(check(
                Level::Error,
                "ROS_AUTOMATIC_DISCOVERY_RANGE",
                format!("`{v}` is not one of SUBNET, LOCALHOST, OFF, SYSTEM_DEFAULT"),
            ));
        } else if distro == Some("humble") {
            out.push(check(
                Level::Note,
                "ROS_AUTOMATIC_DISCOVERY_RANGE",
                "has no effect on Humble (added in Iron)",
            ));
        } else {
            out.push(check(Level::Ok, "ROS_AUTOMATIC_DISCOVERY_RANGE", v));
        }
    }

    // ---- DDS configuration files
    for var in crate::agents::network::DDS_FILE_VARS {
        let Some(v) = get(var) else { continue };
        let for_cyclone = *var == "CYCLONEDDS_URI";
        if let Some(e) = effective {
            let is_cyclone = e.starts_with("rmw_cyclonedds");
            let is_fast = e.starts_with("rmw_fastrtps");
            if (for_cyclone && is_fast) || (!for_cyclone && is_cyclone) {
                out.push(check(
                    Level::Warning,
                    var,
                    format!("is set, but the middleware is `{e}`, which ignores it"),
                ));
            }
        }
        let mut any = false;
        for path in dds_config_paths(v) {
            any = true;
            match std::fs::read_to_string(path) {
                Err(_) => out.push(check(
                    Level::Error,
                    var,
                    format!("`{path}` does not exist or cannot be read"),
                )),
                Ok(text) => match roxmltree::Document::parse(&text) {
                    Err(e) => out.push(check(
                        Level::Error,
                        var,
                        format!("`{path}` is not valid XML: {e}"),
                    )),
                    Ok(doc) => {
                        if !for_cyclone && fastdds_shm_only(&doc) {
                            out.push(
                                check(Level::Warning, var, format!("`{path}` allows only shared-memory transport"))
                                    .help("Nodes on other machines or in containers without a shared /dev/shm will not connect. Add a UDPv4 transport."),
                            );
                        } else {
                            out.push(check(Level::Ok, var, path));
                        }
                    }
                },
            }
        }
        if !any {
            out.push(check(Level::Ok, var, "inline XML"));
        }
    }

    // ---- Zenoh (rmw_zenoh_cpp)
    let zenoh = effective == Some("rmw_zenoh_cpp");
    for var in ZENOH_FILE_VARS.iter().chain(ZENOH_VALUE_VARS) {
        let Some(v) = get(var) else { continue };
        if effective.is_some() && !zenoh {
            out.push(check(
                Level::Warning,
                var,
                format!(
                    "is set, but the middleware is `{}`, which ignores it",
                    effective.unwrap_or("")
                ),
            ));
        }
        if ZENOH_FILE_VARS.contains(var) {
            let path = v.trim_start_matches("file://");
            if Path::new(path).is_file() {
                out.push(check(Level::Ok, var, path));
            } else {
                out.push(check(
                    Level::Error,
                    var,
                    format!("`{path}` does not exist or is not a file; rmw_zenoh_cpp fails to initialize (\"Invalid configuration file\")"),
                ));
            }
        } else if let Some(m) = zenoh_value_problem(var, v) {
            out.push(check(Level::Warning, var, m));
        } else {
            out.push(check(Level::Ok, var, v));
        }
    }
    // With the default session configuration every process connects to a router on
    // localhost:7447; without one, nodes start but never discover each other.
    let custom_connect = get("ZENOH_SESSION_CONFIG_URI").is_some()
        || get("ZENOH_CONFIG_OVERRIDE")
            .is_some_and(|o| o.contains("connect") || o.contains("mode"));
    let skip_check =
        get("ZENOH_ROUTER_CHECK_ATTEMPTS").is_some_and(|v| v.trim_start().starts_with('-'));
    if zenoh && !custom_connect && !skip_check {
        if port_open(ZENOH_ROUTER_ADDR) {
            out.push(check(
                Level::Ok,
                "Zenoh router",
                format!("listening on {ZENOH_ROUTER_ADDR}"),
            ));
        } else {
            out.push(
                check(
                    Level::Warning,
                    "Zenoh router",
                    format!("nothing listens on {ZENOH_ROUTER_ADDR}; nodes using rmw_zenoh_cpp start but do not discover each other"),
                )
                .help("Start a router first: `ros2 run rmw_zenoh_cpp rmw_zenohd`."),
            );
        }
    }
    out
}

/// A Fast DDS profile whose participants can only use shared memory: built-in transports
/// disabled with only SHM user transports, or `builtinTransports` set to SHM.
fn fastdds_shm_only(doc: &roxmltree::Document) -> bool {
    let text = |n: roxmltree::Node| n.text().unwrap_or("").trim().to_string();
    let named = |name: &'static str| {
        doc.descendants()
            .filter(move |n| n.tag_name().name() == name)
    };
    if named("builtinTransports").any(|n| text(n).eq_ignore_ascii_case("SHM")) {
        return true;
    }
    let builtin_off = named("useBuiltinTransports").any(|n| text(n).eq_ignore_ascii_case("false"));
    if !builtin_off {
        return false;
    }
    let types: Vec<String> = named("transport_descriptor")
        .filter_map(|d| {
            d.children()
                .find(|c| c.tag_name().name() == "type")
                .map(text)
        })
        .collect();
    !types.is_empty() && types.iter().all(|t| t.eq_ignore_ascii_case("SHM"))
}

/// Plain-text report, one line per check.
pub fn text(checks: &[Check]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for c in checks {
        let _ = writeln!(
            out,
            "{:<8} {:<31} {}",
            c.level.as_str(),
            c.subject,
            c.message
        );
        if let Some(h) = &c.help {
            let _ = writeln!(out, "{:<8} {:<31} {}", "", "", h);
        }
    }
    let n = |l: Level| checks.iter().filter(|c| c.level == l).count();
    let _ = writeln!(
        out,
        "\n{} error(s), {} warning(s), {} note(s)",
        n(Level::Error),
        n(Level::Warning),
        n(Level::Note)
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn levels(c: &[Check], subject: &str) -> Vec<Level> {
        c.iter()
            .filter(|x| x.subject == subject)
            .map(|x| x.level)
            .collect()
    }

    #[test]
    fn empty_environment_warns_once_per_missing_variable() {
        let c = run(&env(&[]));
        assert_eq!(levels(&c, "ROS_DISTRO"), [Level::Warning]);
        assert_eq!(levels(&c, "AMENT_PREFIX_PATH"), [Level::Warning]);
        assert!(c.iter().all(|x| x.level != Level::Error), "{c:#?}");
    }

    #[test]
    fn middleware_and_domain_errors() {
        let c = run(&env(&[
            ("ROS_DISTRO", "jazzy"),
            ("RMW_IMPLEMENTATION", "rmw_cyclonedds_cp"),
            ("ROS_DOMAIN_ID", "300"),
            ("ROS_AUTOMATIC_DISCOVERY_RANGE", "LOCAL"),
        ]));
        assert_eq!(levels(&c, "RMW_IMPLEMENTATION"), [Level::Error]);
        assert!(c
            .iter()
            .any(|x| x.help.as_deref() == Some("Did you mean `rmw_cyclonedds_cpp`?")));
        assert_eq!(levels(&c, "ROS_DOMAIN_ID"), [Level::Error]);
        assert_eq!(levels(&c, "ROS_AUTOMATIC_DISCOVERY_RANGE"), [Level::Error]);
    }

    #[test]
    fn localhost_only_is_deprecated_on_jazzy_not_humble() {
        let j = run(&env(&[
            ("ROS_DISTRO", "jazzy"),
            ("ROS_LOCALHOST_ONLY", "1"),
        ]));
        assert_eq!(levels(&j, "ROS_LOCALHOST_ONLY"), [Level::Warning]);
        let h = run(&env(&[
            ("ROS_DISTRO", "humble"),
            ("ROS_LOCALHOST_ONLY", "1"),
        ]));
        assert!(levels(&h, "ROS_LOCALHOST_ONLY").is_empty());
    }

    #[test]
    fn dds_files_are_read_and_checked() {
        let dir = std::env::temp_dir().join(format!("rosfix-doctor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let shm = dir.join("shm.xml");
        std::fs::write(&shm, "<profiles><transport_descriptors><transport_descriptor><transport_id>s</transport_id><type>SHM</type></transport_descriptor></transport_descriptors><participant profile_name=\"p\" is_default_profile=\"true\"><rtps><userTransports><transport_id>s</transport_id></userTransports><useBuiltinTransports>false</useBuiltinTransports></rtps></participant></profiles>").unwrap();
        let bad = dir.join("bad.xml");
        std::fs::write(&bad, "<CycloneDDS><Domain>").unwrap();
        let c = run(&env(&[
            ("ROS_DISTRO", "jazzy"),
            ("FASTRTPS_DEFAULT_PROFILES_FILE", shm.to_str().unwrap()),
            ("CYCLONEDDS_URI", &format!("file://{}", bad.display())),
        ]));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            levels(&c, "FASTRTPS_DEFAULT_PROFILES_FILE"),
            [Level::Warning]
        );
        // Cyclone config under the default (Fast DDS) middleware: ignored, and broken.
        assert_eq!(levels(&c, "CYCLONEDDS_URI"), [Level::Warning, Level::Error]);
        let missing = run(&env(&[(
            "FASTDDS_DEFAULT_PROFILES_FILE",
            "/definitely/not/here.xml",
        )]));
        assert_eq!(
            levels(&missing, "FASTDDS_DEFAULT_PROFILES_FILE"),
            [Level::Error]
        );
    }

    #[test]
    fn zenoh_environment() {
        let base = [
            ("ROS_DISTRO", "jazzy"),
            ("RMW_IMPLEMENTATION", "rmw_zenoh_cpp"),
        ];
        let with = |extra: &[(&str, &str)], open: bool| {
            let mut e = env(&base);
            e.extend(env(extra));
            run_with(&e, &move |_| open)
        };
        let c = with(&[], false);
        assert_eq!(levels(&c, "Zenoh router"), [Level::Warning]);
        assert_eq!(levels(&with(&[], true), "Zenoh router"), [Level::Ok]);
        let c = with(
            &[
                ("ZENOH_SESSION_CONFIG_URI", "/definitely/not/here.json5"),
                ("ZENOH_CONFIG_OVERRIDE", "mode=peer"),
                ("ZENOH_ROUTER_CHECK_ATTEMPTS", "forever"),
                ("ZENOH_SHM_MESSAGE_SIZE_THRESHOLD", "1000"),
            ],
            false,
        );
        assert_eq!(levels(&c, "ZENOH_SESSION_CONFIG_URI"), [Level::Error]);
        assert_eq!(levels(&c, "ZENOH_CONFIG_OVERRIDE"), [Level::Warning]);
        assert_eq!(levels(&c, "ZENOH_ROUTER_CHECK_ATTEMPTS"), [Level::Warning]);
        assert_eq!(
            levels(&c, "ZENOH_SHM_MESSAGE_SIZE_THRESHOLD"),
            [Level::Warning]
        );
        // A custom session configuration may not use a local router.
        assert!(levels(&c, "Zenoh router").is_empty());
        let ok = with(
            &[
                (
                    "ZENOH_CONFIG_OVERRIDE",
                    "scouting/multicast/enabled=true;listen/endpoints=[\"tcp/0.0.0.0:7448\"]",
                ),
                ("ZENOH_SHM_MESSAGE_SIZE_THRESHOLD", "4096"),
            ],
            true,
        );
        assert!(
            ok.iter()
                .filter(|x| x.subject.starts_with("Z"))
                .all(|x| x.level == Level::Ok),
            "{ok:#?}"
        );
        // Zenoh variables under a DDS middleware do nothing.
        let dds = run_with(
            &env(&[
                ("ROS_DISTRO", "jazzy"),
                ("ZENOH_ROUTER_CHECK_ATTEMPTS", "3"),
            ]),
            &|_| false,
        );
        assert_eq!(
            levels(&dds, "ZENOH_ROUTER_CHECK_ATTEMPTS"),
            [Level::Warning, Level::Ok]
        );
    }

    #[test]
    fn healthy_environment_has_no_problems() {
        let dir = std::env::temp_dir();
        let c = run(&env(&[
            ("ROS_DISTRO", "kilted"),
            ("AMENT_PREFIX_PATH", dir.to_str().unwrap()),
            ("ROS_DOMAIN_ID", "42"),
            ("ROS_AUTOMATIC_DISCOVERY_RANGE", "LOCALHOST"),
        ]));
        assert!(c.iter().all(|x| x.level == Level::Ok), "{c:#?}");
    }
}
