//! `rosfix check-bag`: the static launch graph against what actually ran.
//!
//! Static analysis sees what the files say; a recording shows what the running system did.
//! The comparison reads a rosbag2 `metadata.yaml` (topics, types, message counts and the QoS
//! the publishers offered) or the output of `ros2 topic list -t` (topics and types), and
//! reports where the two disagree:
//!
//! - a topic whose recorded type differs from the type the code publishes or subscribes;
//! - a topic whose recorded publishers offered a different reliability or durability than
//!   the code declares (a QoS override, a middleware profile, or another publisher);
//! - a recorded topic that no node of the launch graph publishes (a publisher outside the
//!   workspace, or a node rosfix could not resolve);
//! - with a launch file selected: a topic that launch file publishes but the recording does
//!   not contain (the publisher did not run, or the topic was not recorded).

use crate::engine::Analysis;
use crate::extract::yaml::{self, YNode};
use crate::facts::{Durability, EndpointKind, Reliability};
use crate::resolve::instance_endpoints;
use crate::source::Loc;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct Observed {
    pub topic: String,
    pub msg_type: String,
    pub messages: Option<u64>,
    /// (reliability, durability) offered by each recorded publisher.
    #[serde(skip)]
    pub offered: Vec<(Option<Reliability>, Option<Durability>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    TypeMismatch,
    QosDiffers,
    NoStaticPublisher,
    NotRecorded,
}

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub kind: Kind,
    pub topic: String,
    pub message: String,
    /// Where the code declares the endpoint (file:line), when the item is about one.
    pub location: Option<String>,
}

fn reliability(v: &str) -> Option<Reliability> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "reliable" => Some(Reliability::Reliable),
        "2" | "best_effort" => Some(Reliability::BestEffort),
        _ => None,
    }
}

fn durability(v: &str) -> Option<Durability> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "transient_local" => Some(Durability::TransientLocal),
        "2" | "volatile" => Some(Durability::Volatile),
        _ => None,
    }
}

fn offered(n: Option<&YNode>) -> Vec<(Option<Reliability>, Option<Durability>)> {
    let profiles: Vec<YNode> = match n {
        // rosbag2 stores the list as a YAML document inside a string.
        Some(YNode::Scalar { value, .. }) => yaml::parse(value).ok().and_then(|d| d.into_iter().next()).map(|d| d.items().to_vec()).unwrap_or_default(),
        Some(seq) => seq.items().to_vec(),
        None => Vec::new(),
    };
    profiles
        .iter()
        .map(|p| {
            (
                p.get("reliability").and_then(YNode::as_str).and_then(reliability),
                p.get("durability").and_then(YNode::as_str).and_then(durability),
            )
        })
        .collect()
}

/// Reads a rosbag2 `metadata.yaml`, or `ros2 topic list -t` output (`/name [pkg/msg/Type]`).
pub fn read_observed(text: &str) -> Result<Vec<Observed>, String> {
    if text.contains("rosbag2_bagfile_information") {
        let docs = yaml::parse(text)?;
        let info = docs
            .first()
            .and_then(|d| d.get("rosbag2_bagfile_information"))
            .ok_or("not a rosbag2 metadata file")?;
        let mut out = Vec::new();
        for t in info.get("topics_with_message_count").map(YNode::items).unwrap_or_default() {
            let Some(m) = t.get("topic_metadata") else { continue };
            let (Some(name), Some(ty)) = (m.get("name").and_then(YNode::as_str), m.get("type").and_then(YNode::as_str)) else {
                continue;
            };
            out.push(Observed {
                topic: name.to_string(),
                msg_type: ty.to_string(),
                messages: t.get("message_count").and_then(YNode::as_str).and_then(|c| c.trim().parse().ok()),
                offered: offered(m.get("offered_qos_profiles")),
            });
        }
        return Ok(out);
    }
    let mut out = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| l.starts_with('/')) {
        let (name, ty) = match line.split_once(' ') {
            Some((n, rest)) => (n, rest.trim().trim_start_matches('[').trim_end_matches(']').to_string()),
            None => (line, String::new()),
        };
        out.push(Observed { topic: name.to_string(), msg_type: ty, messages: None, offered: Vec::new() });
    }
    if out.is_empty() {
        return Err("neither a rosbag2 metadata.yaml nor `ros2 topic list -t` output".into());
    }
    Ok(out)
}

/// `pkg/msg/Type`, whatever the source wrote (`pkg::msg::Type`, or only `Type` from Python).
fn norm_type(t: &str) -> String {
    t.trim().replace("::", "/")
}

fn same_type(a: &str, b: &str) -> bool {
    let (a, b) = (norm_type(a), norm_type(b));
    if a.contains('/') && b.contains('/') {
        return a == b;
    }
    // One side is only a class name: compare what both know.
    a.rsplit('/').next() == b.rsplit('/').next()
}

struct StaticEp {
    kind: EndpointKind,
    msg_type: Option<String>,
    reliability: Option<Reliability>,
    durability: Option<Durability>,
    loc: Loc,
    node: String,
}

/// The one value every recorded publisher offered, if they agree and it is known.
fn uniform<T: Copy + PartialEq>(v: Vec<Option<T>>) -> Option<T> {
    let first = *v.first()?;
    v.iter().all(|x| *x == first).then_some(first).flatten()
}

pub fn compare(a: &Analysis, observed: &[Observed], launch: Option<&str>) -> Result<Vec<Item>, String> {
    let roots: Vec<_> = match launch {
        None => a.graph.roots.iter().collect(),
        Some(l) => {
            let l = l.replace('\\', "/");
            let r: Vec<_> = a.graph.roots.iter().filter(|r| a.index.file(r.root).rel_path.ends_with(&l)).collect();
            if r.is_empty() {
                return Err(format!("no launch graph for `{l}` (see `rosfix graph`)"));
            }
            r
        }
    };
    let mut by_topic: BTreeMap<String, Vec<StaticEp>> = BTreeMap::new();
    for r in &roots {
        for n in &r.nodes {
            for e in instance_endpoints(&a.index, n).0.into_iter().filter(|e| e.kind.is_topic()) {
                by_topic.entry(e.name.clone()).or_default().push(StaticEp {
                    kind: e.kind,
                    msg_type: e.msg_type.clone(),
                    reliability: e.qos.reliability,
                    durability: e.qos.durability,
                    loc: e.loc,
                    node: n.display_name(),
                });
            }
        }
    }
    let at = |loc: Loc| {
        let f = a.index.file(loc.file);
        format!("{}:{}", f.rel_path, f.line_col(loc.start).0)
    };
    let mut out = Vec::new();
    for o in observed {
        let eps = by_topic.get(&o.topic).map(Vec::as_slice).unwrap_or_default();
        for e in eps {
            if let Some(t) = &e.msg_type {
                if !o.msg_type.is_empty() && !same_type(t, &o.msg_type) {
                    out.push(Item {
                        kind: Kind::TypeMismatch,
                        topic: o.topic.clone(),
                        message: format!("`{}` uses `{}`, but the recording has `{}`", e.node, norm_type(t), o.msg_type),
                        location: Some(at(e.loc)),
                    });
                }
            }
        }
        let pubs: Vec<&StaticEp> = eps.iter().filter(|e| e.kind == EndpointKind::Publisher).collect();
        if pubs.is_empty() {
            out.push(Item {
                kind: Kind::NoStaticPublisher,
                topic: o.topic.clone(),
                message: "recorded, but no node of the launch graph publishes it (a publisher outside the workspace, or one rosfix could not resolve)".into(),
                location: None,
            });
            continue;
        }
        // QoS: only when every recorded publisher offered the same policy and every static
        // publisher declares one that differs from it.
        let rec_rel = uniform(o.offered.iter().map(|x| x.0).collect());
        let rec_dur = uniform(o.offered.iter().map(|x| x.1).collect());
        for p in &pubs {
            let mut diffs = Vec::new();
            if let (Some(r), Some(s)) = (rec_rel, p.reliability) {
                if r != s {
                    diffs.push(format!("reliability {s:?} in code, {r:?} recorded"));
                }
            }
            if let (Some(r), Some(s)) = (rec_dur, p.durability) {
                if r != s {
                    diffs.push(format!("durability {s:?} in code, {r:?} recorded"));
                }
            }
            if !diffs.is_empty() {
                out.push(Item {
                    kind: Kind::QosDiffers,
                    topic: o.topic.clone(),
                    message: format!(
                        "`{}`: {}; a QoS override, a middleware profile or another publisher changed it",
                        p.node,
                        diffs.join(", ")
                    ),
                    location: Some(at(p.loc)),
                });
            }
        }
    }
    if launch.is_some() {
        for (topic, eps) in &by_topic {
            if observed.iter().any(|o| &o.topic == topic) {
                continue;
            }
            if let Some(p) = eps.iter().find(|e| e.kind == EndpointKind::Publisher) {
                out.push(Item {
                    kind: Kind::NotRecorded,
                    topic: topic.clone(),
                    message: format!("`{}` publishes it, but the recording does not contain it (the node did not run, or the topic was not recorded)", p.node),
                    location: Some(at(p.loc)),
                });
            }
        }
    }
    out.sort_by(|x, y| (x.kind, &x.topic).cmp(&(y.kind, &y.topic)));
    out.dedup_by(|x, y| x.kind == y.kind && x.topic == y.topic && x.message == y.message);
    Ok(out)
}

pub fn text(items: &[Item], observed: usize) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for i in items {
        let label = match i.kind {
            Kind::TypeMismatch => "type",
            Kind::QosDiffers => "qos",
            Kind::NoStaticPublisher => "unknown",
            Kind::NotRecorded => "missing",
        };
        let _ = writeln!(s, "{label:<8} {}: {}", i.topic, i.message);
        if let Some(l) = &i.location {
            let _ = writeln!(s, "{:<8}   --> {l}", "");
        }
    }
    let n = |k: Kind| items.iter().filter(|i| i.kind == k).count();
    let _ = writeln!(
        s,
        "\n{observed} recorded topic(s): {} type mismatch(es), {} QoS difference(s), {} without a static publisher, {} not recorded",
        n(Kind::TypeMismatch),
        n(Kind::QosDiffers),
        n(Kind::NoStaticPublisher),
        n(Kind::NotRecorded)
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_rosbag2_metadata_and_topic_lists() {
        let v5 = "rosbag2_bagfile_information:\n  version: 5\n  topics_with_message_count:\n    - topic_metadata:\n        name: /scan\n        type: sensor_msgs/msg/LaserScan\n        serialization_format: cdr\n        offered_qos_profiles: \"- history: 3\\n  depth: 0\\n  reliability: 2\\n  durability: 2\\n\"\n      message_count: 1079\n";
        let o = read_observed(v5).unwrap();
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].messages, Some(1079));
        assert_eq!(o[0].offered, [(Some(Reliability::BestEffort), Some(Durability::Volatile))]);
        let list = read_observed("/chatter [std_msgs/msg/String]\n/rosout [rcl_interfaces/msg/Log]\n").unwrap();
        assert_eq!(list[0].msg_type, "std_msgs/msg/String");
        assert!(read_observed("hello").is_err());
    }
}
