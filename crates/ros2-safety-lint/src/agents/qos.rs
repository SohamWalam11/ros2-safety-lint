//! QoS agent: publisher/subscription compatibility across the launch graph, and
//! `qos_overrides` that silently do nothing.

use super::{Agent, Context};
use crate::catalog::{known_publishers, LATCHED_TOPICS};
use crate::diag::Diagnostic;
use crate::facts::*;
use crate::resolve::{
    expand_name, instance_endpoints, resolve_path, ImplStatus, NodeInstance, PathRes,
};
use crate::source::Loc;
use std::collections::{HashMap, HashSet};

pub struct QosAgent;

#[derive(Clone)]
struct Ep {
    kind: EndpointKind,
    topic: String,
    qos: QosSpec,
    loc: Loc,
    owner: String,
}

impl Agent for QosAgent {
    fn id(&self) -> &'static str {
        "qos"
    }
    fn title(&self) -> &'static str {
        "QoS compatibility and overrides"
    }
    fn inputs(&self) -> &'static [crate::workspace::FileKind] {
        use crate::workspace::FileKind as K;
        // The launch graph plus RViz displays (they subscribe too).
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
            K::Rviz,
        ]
    }
    fn relevant(&self, cx: &Context) -> bool {
        !cx.graph.roots.is_empty()
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        let mut seen: HashSet<(Loc, Loc, &'static str)> = HashSet::new();
        for root in &cx.graph.roots {
            let mut eps: Vec<Ep> = Vec::new();
            for n in &root.nodes {
                let (resolved, complete) = instance_endpoints(cx.ix, n);
                let mut resolved = resolved;
                overrides(cx, n, &mut resolved, complete, out);
                for e in resolved.into_iter().filter(|e| e.kind.is_topic()) {
                    eps.push(Ep {
                        kind: e.kind,
                        topic: e.name,
                        qos: e.qos,
                        loc: e.loc,
                        owner: n.display_name(),
                    });
                }
                eps.extend(catalog_endpoints(n));
                eps.extend(rviz_endpoints(cx, n));
            }
            if let Some(profiles) = fastdds_profiles(cx, root) {
                for e in eps.iter_mut().filter(|e| e.qos.system_default) {
                    profiles.apply(e);
                }
            }
            let mut by_topic: HashMap<&str, (Vec<&Ep>, Vec<&Ep>)> = HashMap::new();
            for e in &eps {
                let slot = by_topic.entry(e.topic.as_str()).or_default();
                if e.kind == EndpointKind::Publisher {
                    slot.0.push(e);
                } else {
                    slot.1.push(e);
                }
            }
            for (topic, (pubs, subs)) in &by_topic {
                for s in subs {
                    for p in pubs {
                        for (rule, msg) in incompatibilities(p, s) {
                            if seen.insert((s.loc, p.loc, rule)) {
                                out.push(
                                    Diagnostic::new(
                                        rule,
                                        s.loc,
                                        format!("`{}` on `{topic}`: {msg}", s.owner),
                                    )
                                    .related(
                                        p.loc,
                                        format!("publisher `{}`: {}", p.owner, p.qos.origin),
                                    ),
                                );
                            }
                        }
                    }
                    // Late joiner on a latched topic.
                    let all_latched = !pubs.is_empty()
                        && pubs
                            .iter()
                            .all(|p| p.qos.durability == Some(Durability::TransientLocal));
                    let leaf = topic.rsplit('/').next().unwrap_or(topic);
                    if all_latched
                        && s.qos.durability == Some(Durability::Volatile)
                        && LATCHED_TOPICS.contains(&leaf)
                        && seen.insert((s.loc, s.loc, "Q004"))
                    {
                        out.push(
                            Diagnostic::new("Q004", s.loc, format!("`{}` subscribes to latched topic `{topic}` with VOLATILE durability and will miss messages published before it started", s.owner))
                                .related(pubs[0].loc, "published as TRANSIENT_LOCAL here"),
                        );
                    }
                }
            }
        }
    }
}

/// Writer/reader QoS from Fast DDS XML profiles: reliability and durability by profile name
/// (a topic name), plus the default profile of each side.
#[derive(Default)]
struct DdsProfiles {
    file: String,
    /// (is_writer, profile name or "" for the default profile) → (reliability, durability)
    entries: HashMap<(bool, String), (Option<Reliability>, Option<Durability>)>,
}

impl DdsProfiles {
    /// Fills the policies a system-default endpoint left to the middleware. rmw_fastrtps
    /// honours every policy set in ROS code and uses the XML only for the ones left at
    /// SYSTEM_DEFAULT; a topic profile (named after the topic) wins over the default one.
    fn apply(&self, e: &mut Ep) {
        let writer = e.kind == EndpointKind::Publisher;
        let bare = e.topic.trim_start_matches('/').to_string();
        let found = [e.topic.clone(), bare, String::new()]
            .into_iter()
            .find_map(|name| {
                self.entries
                    .get(&(writer, name.clone()))
                    .map(|v| (name, *v))
            });
        let Some((name, (rel, dur))) = found else {
            return;
        };
        let mut used = false;
        if e.qos.reliability.is_none() && rel.is_some() {
            e.qos.reliability = rel;
            used = true;
        }
        if e.qos.durability.is_none() && dur.is_some() {
            e.qos.durability = dur;
            used = true;
        }
        if used {
            let which = if name.is_empty() {
                "default profile".to_string()
            } else {
                format!("profile `{name}`")
            };
            e.qos.origin = format!(
                "{} with the Fast DDS {which} of {}",
                e.qos.origin, self.file
            );
        }
    }
}

/// The Fast DDS XML profiles a launch graph points its nodes at: a workspace file named by
/// `FASTRTPS_DEFAULT_PROFILES_FILE` / `FASTDDS_DEFAULT_PROFILES_FILE` in the launch
/// environment, unless the graph selects a middleware other than Fast DDS.
fn fastdds_profiles(cx: &Context, root: &crate::resolve::RootGraph) -> Option<DdsProfiles> {
    let value = |name: &str| {
        root.env
            .iter()
            .rev()
            .find(|e| e.name == name)
            .and_then(|e| e.value.clone())
    };
    if value("RMW_IMPLEMENTATION").is_some_and(|r| !r.trim().starts_with("rmw_fastrtps")) {
        return None;
    }
    let v = value("FASTDDS_DEFAULT_PROFILES_FILE")
        .or_else(|| value("FASTRTPS_DEFAULT_PROFILES_FILE"))?;
    let path = crate::agents::network::dds_config_paths(&v)
        .next()?
        .to_string();
    // XML profiles are not indexed unless something else reads them: read from disk then.
    let (rel, text) = match resolve_path(cx.ix, &path) {
        PathRes::Found(f) => (cx.ix.file(f).rel_path.clone(), cx.ix.file(f).text.clone()),
        PathRes::Exists(rel) if !rel.starts_with('<') => {
            let text = std::fs::read_to_string(cx.ix.root.join(&rel)).ok()?;
            (rel, text)
        }
        _ => return None,
    };
    let doc = roxmltree::Document::parse(&text).ok()?;
    let mut p = DdsProfiles {
        file: rel,
        ..Default::default()
    };
    let text = |n: roxmltree::Node, tag: &str| {
        n.descendants()
            .find(|d| d.tag_name().name() == tag)
            .and_then(|d| d.children().find(|c| c.tag_name().name() == "kind"))
            .and_then(|k| k.text())
            .map(|t| t.trim().to_string())
    };
    for n in doc.descendants() {
        let writer = match n.tag_name().name() {
            "publisher" | "data_writer" => true,
            "subscriber" | "data_reader" => false,
            _ => continue,
        };
        let rel = text(n, "reliability").and_then(|k| match k.as_str() {
            "RELIABLE_RELIABILITY_QOS" | "RELIABLE" => Some(Reliability::Reliable),
            "BEST_EFFORT_RELIABILITY_QOS" | "BEST_EFFORT" => Some(Reliability::BestEffort),
            _ => None,
        });
        let dur = text(n, "durability").and_then(|k| match k.as_str() {
            "VOLATILE_DURABILITY_QOS" | "VOLATILE" => Some(Durability::Volatile),
            "TRANSIENT_LOCAL_DURABILITY_QOS" | "TRANSIENT_LOCAL" => {
                Some(Durability::TransientLocal)
            }
            _ => None,
        });
        if rel.is_none() && dur.is_none() {
            continue;
        }
        if n.attribute("is_default_profile") == Some("true") {
            p.entries.insert((writer, String::new()), (rel, dur));
        }
        if let Some(name) = n.attribute("profile_name") {
            p.entries.insert((writer, name.to_string()), (rel, dur));
        }
    }
    (!p.entries.is_empty()).then_some(p)
}

fn incompatibilities(p: &Ep, s: &Ep) -> Vec<(&'static str, String)> {
    let mut v = Vec::new();
    if let (Some(Reliability::BestEffort), Some(Reliability::Reliable)) =
        (p.qos.reliability, s.qos.reliability)
    {
        v.push(("Q001", format!("subscription requests RELIABLE ({}) but publisher `{}` offers BEST_EFFORT — no messages will be delivered", s.qos.origin, p.owner)));
    }
    if let (Some(Durability::Volatile), Some(Durability::TransientLocal)) =
        (p.qos.durability, s.qos.durability)
    {
        v.push(("Q002", format!("subscription requests TRANSIENT_LOCAL ({}) but publisher `{}` offers VOLATILE — no messages will be delivered", s.qos.origin, p.owner)));
    }
    if let (Some(pd), Some(sd)) = (p.qos.deadline_ns, s.qos.deadline_ns) {
        if pd > sd {
            v.push((
                "Q003",
                format!(
                    "publisher `{}` offers a longer deadline than the subscription requests",
                    p.owner
                ),
            ));
        }
    }
    if let (Some(LivelinessKind::Automatic), Some(LivelinessKind::ManualByTopic)) =
        (p.qos.liveliness, s.qos.liveliness)
    {
        v.push((
            "Q003",
            format!(
                "publisher `{}` offers AUTOMATIC liveliness but MANUAL_BY_TOPIC is requested",
                p.owner
            ),
        ));
    }
    if let (Some(pl), Some(sl)) = (p.qos.lease_ns, s.qos.lease_ns) {
        if pl > sl {
            v.push((
                "Q003",
                format!(
                    "publisher `{}` offers a longer liveliness lease than requested",
                    p.owner
                ),
            ));
        }
    }
    v
}

/// Applies `qos_overrides.<topic>.<publisher|subscription>.<policy>` and reports ignored ones.
fn overrides(
    _cx: &Context,
    n: &NodeInstance,
    eps: &mut [crate::resolve::ResolvedEndpoint],
    complete: bool,
    out: &mut Vec<Diagnostic>,
) {
    if n.impl_status != ImplStatus::Resolved {
        return;
    }
    let node_name = n.name.clone().unwrap_or_default();
    for b in &n.params {
        let Some(rest) = b.name.strip_prefix("qos_overrides.") else {
            continue;
        };
        // The topic itself contains dots only rarely; split from the right.
        let mut parts: Vec<&str> = rest.rsplitn(3, '.').collect();
        if parts.len() != 3 {
            continue;
        }
        parts.reverse();
        let (topic, side, policy) = (parts[0], parts[1], parts[2]);
        let kind = match side {
            "publisher" => EndpointKind::Publisher,
            "subscription" => EndpointKind::Subscription,
            _ => continue,
        };
        let topic_fq = expand_name(topic, &n.namespace, &node_name);
        let targets: Vec<usize> = eps
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == kind && e.name == topic_fq)
            .map(|(i, _)| i)
            .collect();
        if targets.is_empty() {
            if complete {
                out.push(Diagnostic::new(
                    "Q005",
                    b.key_loc,
                    format!("`qos_overrides` for {side} `{topic}` has no effect: node `{}` has no {side} on that topic", n.display_name()),
                ));
            }
            continue;
        }
        for i in targets {
            if !eps[i].qos_overrides_enabled {
                out.push(
                    Diagnostic::new(
                        "Q005",
                        b.key_loc,
                        format!(
                            "`qos_overrides` for {side} `{topic}` is ignored: the {side} is created without `QosOverridingOptions`, so the code's QoS ({}) wins",
                            eps[i].qos.origin
                        ),
                    )
                    .related(eps[i].loc, format!("{side} created here")),
                );
                continue;
            }
            let val = b
                .value
                .as_ref()
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let q = &mut eps[i].qos;
            match policy {
                "reliability" => {
                    q.reliability = if val == "best_effort" {
                        Some(Reliability::BestEffort)
                    } else if val == "reliable" {
                        Some(Reliability::Reliable)
                    } else {
                        None
                    }
                }
                "durability" => {
                    q.durability = if val == "transient_local" {
                        Some(Durability::TransientLocal)
                    } else if val == "volatile" {
                        Some(Durability::Volatile)
                    } else {
                        None
                    }
                }
                "depth" => q.depth = b.value.as_ref().and_then(|v| v.as_f64()).map(|d| d as u32),
                _ => {}
            }
            q.origin = format!("{} + qos_overrides", q.origin);
        }
    }
}

fn catalog_endpoints(n: &NodeInstance) -> Vec<Ep> {
    if n.impl_status == ImplStatus::Resolved {
        return Vec::new();
    }
    let (Some(p), Some(e)) = (n.pkg.as_deref(), n.exec.as_deref()) else {
        return Vec::new();
    };
    let node_name = n.name.clone().unwrap_or_default();
    known_publishers(p, e)
        .into_iter()
        .map(|(t, q)| Ep {
            kind: EndpointKind::Publisher,
            topic: expand_name(t, &n.namespace, &node_name),
            qos: q,
            loc: n.decl.loc,
            owner: n.display_name(),
        })
        .collect()
}

fn rviz_endpoints(cx: &Context, n: &NodeInstance) -> Vec<Ep> {
    if n.pkg.as_deref() != Some("rviz2") {
        return Vec::new();
    }
    let mut out = Vec::new();
    let args: Vec<&str> = n
        .arguments
        .iter()
        .map(|a| a.as_deref().unwrap_or(""))
        .collect();
    for (i, a) in args.iter().enumerate() {
        let path = if *a == "-d" || *a == "--display-config" {
            args.get(i + 1).copied()
        } else {
            a.strip_prefix("-d").filter(|s| !s.is_empty())
        };
        let Some(path) = path else { continue };
        if let PathRes::Found(fid) = resolve_path(cx.ix, path.trim()) {
            for s in cx.ix.rviz.get(&fid).map(|v| v.as_slice()).unwrap_or(&[]) {
                out.push(Ep {
                    kind: EndpointKind::Subscription,
                    topic: expand_name(
                        &s.topic,
                        &n.namespace,
                        n.name.as_deref().unwrap_or("rviz2"),
                    ),
                    qos: s.qos.clone(),
                    loc: s.loc,
                    owner: format!("RViz display {}", s.display),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::testutil::analyze_files;

    const PKG: (&str, &str) = (
        "p/package.xml",
        "<package format=\"3\"><name>p</name></package>",
    );
    const CMAKE: (&str, &str) = ("p/CMakeLists.txt", "project(p)\nadd_executable(drv src/d.cpp)\nadd_executable(user src/u.cpp)\ninstall(TARGETS drv user DESTINATION lib/${PROJECT_NAME})\n");

    #[test]
    fn reliability_mismatch_and_ignored_override() {
        let d = analyze_files(&[
            PKG,
            CMAKE,
            ("p/src/d.cpp", "class D : public rclcpp::Node { public: D() : Node(\"driver\") { pub_ = create_publisher<sensor_msgs::msg::LaserScan>(\"scan\", rclcpp::SensorDataQoS()); } };\n"),
            ("p/src/u.cpp", "class U : public rclcpp::Node { public: U() : Node(\"user\") { sub_ = create_subscription<sensor_msgs::msg::LaserScan>(\"scan\", 10, [](sensor_msgs::msg::LaserScan::SharedPtr) {}); } };\n"),
            ("p/config/u.yaml", "user:\n  ros__parameters:\n    qos_overrides:\n      /scan:\n        subscription:\n          reliability: best_effort\n"),
            ("p/launch/a.launch.xml", "<launch><node pkg=\"p\" exec=\"drv\"/><node pkg=\"p\" exec=\"user\"><param from=\"$(find-pkg-share p)/config/u.yaml\"/></node></launch>"),
        ]);
        assert!(d.iter().any(|x| x.rule == "Q001"), "{d:#?}");
        assert!(d.iter().any(|x| x.rule == "Q005"), "{d:#?}");
    }

    #[test]
    fn override_enabled_fixes_mismatch() {
        let d = analyze_files(&[
            PKG,
            CMAKE,
            ("p/src/d.cpp", "class D : public rclcpp::Node { public: D() : Node(\"driver\") { pub_ = create_publisher<sensor_msgs::msg::LaserScan>(\"scan\", rclcpp::SensorDataQoS()); } };\n"),
            ("p/src/u.cpp", "class U : public rclcpp::Node { public: U() : Node(\"user\") { rclcpp::SubscriptionOptions o; o.qos_overriding_options = rclcpp::QosOverridingOptions::with_default_policies(); sub_ = create_subscription<sensor_msgs::msg::LaserScan>(\"scan\", 10, [](sensor_msgs::msg::LaserScan::SharedPtr) {}, o); } };\n"),
            ("p/config/u.yaml", "user:\n  ros__parameters:\n    qos_overrides:\n      /scan:\n        subscription:\n          reliability: best_effort\n"),
            ("p/launch/a.launch.xml", "<launch><node pkg=\"p\" exec=\"drv\"/><node pkg=\"p\" exec=\"user\"><param from=\"$(find-pkg-share p)/config/u.yaml\"/></node></launch>"),
        ]);
        assert!(
            !d.iter().any(|x| x.rule == "Q001" || x.rule == "Q005"),
            "{d:#?}"
        );
    }

    /// A system-default subscription takes its reliability from the middleware: unknown
    /// without a profile, decided by a Fast DDS profile when the launch graph points at one.
    #[test]
    fn system_default_qos_follows_the_fastdds_profile() {
        let drv = ("p/src/d.cpp", "class D : public rclcpp::Node { public: D() : Node(\"driver\") { pub_ = create_publisher<sensor_msgs::msg::LaserScan>(\"scan\", rclcpp::SensorDataQoS()); } };\n");
        let user = ("p/src/u.cpp", "class U : public rclcpp::Node { public: U() : Node(\"user\") { sub_ = create_subscription<sensor_msgs::msg::LaserScan>(\"scan\", rclcpp::SystemDefaultsQoS(), [](sensor_msgs::msg::LaserScan::SharedPtr) {}); } };\n");
        let plain =
            "<launch><node pkg=\"p\" exec=\"drv\"/><node pkg=\"p\" exec=\"user\"/></launch>";
        let d = analyze_files(&[PKG, CMAKE, drv, user, ("p/launch/a.launch.xml", plain)]);
        assert!(
            d.iter().all(|x| x.rule != "Q001"),
            "no verdict without a profile: {d:#?}"
        );

        let profile = "<?xml version=\"1.0\"?><profiles xmlns=\"http://www.eprosima.com\"><data_reader profile_name=\"/scan\"><qos><reliability><kind>RELIABLE_RELIABILITY_QOS</kind></reliability></qos></data_reader></profiles>";
        let with = |rmw: &str| {
            format!("<launch><set_env name=\"FASTRTPS_DEFAULT_PROFILES_FILE\" value=\"$(find-pkg-share p)/config/fastdds.xml\"/>{rmw}<node pkg=\"p\" exec=\"drv\"/><node pkg=\"p\" exec=\"user\"/></launch>")
        };
        let fast = with("");
        let d = analyze_files(&[
            PKG,
            CMAKE,
            drv,
            user,
            ("p/config/fastdds.xml", profile),
            ("p/launch/a.launch.xml", fast.as_str()),
        ]);
        let q = d
            .iter()
            .find(|x| x.rule == "Q001")
            .unwrap_or_else(|| panic!("{d:#?}"));
        assert!(
            q.message.contains("`user`") || q.message.contains("user"),
            "{q:#?}"
        );
        // Another middleware ignores the Fast DDS file.
        let cyclone = with("<set_env name=\"RMW_IMPLEMENTATION\" value=\"rmw_cyclonedds_cpp\"/>");
        let d = analyze_files(&[
            PKG,
            CMAKE,
            drv,
            user,
            ("p/config/fastdds.xml", profile),
            ("p/launch/a.launch.xml", cyclone.as_str()),
        ]);
        assert!(d.iter().all(|x| x.rule != "Q001"), "{d:#?}");
    }
}
