//! RViz2 config files: each enabled display's subscription topic and QoS.

use super::yaml::{self, YNode};
use crate::facts::{Durability, QosSpec, Reliability, RvizSubscription};
use crate::source::{FileId, Loc};

pub fn extract(file: FileId, text: &str) -> Result<Vec<RvizSubscription>, String> {
    let docs = yaml::parse(text)?;
    let mut out = Vec::new();
    for d in &docs {
        if let Some(vm) = d.get("Visualization Manager") {
            walk_displays(file, vm, true, &mut out);
        }
    }
    Ok(out)
}

fn walk_displays(
    file: FileId,
    node: &YNode,
    enabled_parent: bool,
    out: &mut Vec<RvizSubscription>,
) {
    for disp in node.get("Displays").map(|d| d.items()).unwrap_or(&[]) {
        let enabled = enabled_parent
            && disp
                .get("Enabled")
                .and_then(|e| e.as_str())
                .is_none_or(|e| e == "true");
        let class = disp.get("Class").and_then(|c| c.as_str()).unwrap_or("");
        if class.ends_with("/Group") {
            walk_displays(file, disp, enabled, out);
            continue;
        }
        if !enabled {
            continue;
        }
        for key in ["Topic", "Description Topic", "Update Topic"] {
            let Some(t) = disp.get(key) else { continue };
            let Some(v) = t.get("Value").and_then(|v| v.as_str()) else {
                continue;
            };
            if v.is_empty() {
                continue;
            }
            let mut q = QosSpec::ros_default(format!("RViz {class} display"));
            if let Some(r) = t.get("Reliability Policy").and_then(|r| r.as_str()) {
                q.reliability = match r {
                    "Best Effort" => Some(Reliability::BestEffort),
                    "Reliable" => Some(Reliability::Reliable),
                    _ => None, // "System Default"
                };
            }
            if let Some(r) = t.get("Durability Policy").and_then(|r| r.as_str()) {
                q.durability = match r {
                    "Transient Local" => Some(Durability::TransientLocal),
                    "Volatile" => Some(Durability::Volatile),
                    _ => None,
                };
            }
            let (_, vnode) = t.entry("Value").unwrap();
            out.push(RvizSubscription {
                topic: v.to_string(),
                display: format!(
                    "{} ({})",
                    disp.get("Name").and_then(|n| n.as_str()).unwrap_or(class),
                    key
                ),
                qos: q,
                loc: Loc::new(file, vnode.start(), vnode.end()),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays() {
        let src = "Visualization Manager:\n  Displays:\n    - Class: rviz_default_plugins/LaserScan\n      Enabled: true\n      Name: LaserScan\n      Topic:\n        Value: /scan\n        Reliability Policy: Reliable\n        Durability Policy: Volatile\n    - Class: rviz_default_plugins/Map\n      Enabled: false\n      Topic:\n        Value: /map\n";
        let s = extract(0, src).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].topic, "/scan");
        assert_eq!(s[0].qos.reliability, Some(Reliability::Reliable));
    }
}
