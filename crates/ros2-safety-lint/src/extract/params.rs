//! Parameter YAML files: node sections → flattened parameters with spans.

use super::yaml::YNode;
use crate::facts::{MissingRosParams, ParamEntry, ParamFile, ParamSection};
use crate::source::{FileId, Loc};
use crate::util::edit_distance;

pub const ROS_PARAMETERS: &str = "ros__parameters";

/// True if the text plausibly is a ROS parameter file: some mapping key is
/// `ros__parameters` or a misspelling of it that still says `ros` (`ros__parametres`,
/// `ros_parameters`, `ros__params`). A file whose only section is misspelled must count,
/// or P002 could never report it.
pub fn looks_like_param_file(text: &str) -> bool {
    text.contains(ROS_PARAMETERS)
        || text.lines().any(|l| {
            let key = l.trim_start().split(':').next().unwrap_or("").trim();
            key.to_ascii_lowercase().contains("ros") && is_misspelling(key)
        })
}

fn is_misspelling(key: &str) -> bool {
    key != ROS_PARAMETERS
        && (edit_distance(key, ROS_PARAMETERS) <= 3 || key == "parameters" || key == "ros_params")
        && key.contains("param")
}

fn is_leafish(node: &YNode) -> bool {
    match node {
        YNode::Scalar { .. } | YNode::Seq { .. } => true,
        YNode::Map { entries, .. } => entries.iter().all(|(_, v)| !v.is_map()),
    }
}

pub fn extract(file: FileId, docs: &[YNode]) -> ParamFile {
    let mut pf = ParamFile {
        file,
        sections: Vec::new(),
        missing_ros_parameters: Vec::new(),
    };
    for doc in docs {
        for (k, v) in doc.entries() {
            if let Some(key) = k.as_str() {
                walk(
                    file,
                    vec![key.to_string()],
                    Loc::new(file, k.start(), k.end()),
                    v,
                    &mut pf,
                );
            }
        }
    }
    pf
}

fn walk(file: FileId, path: Vec<String>, key_loc: Loc, node: &YNode, pf: &mut ParamFile) {
    let YNode::Map { entries, .. } = node else {
        // A scalar directly under a top-level key: that key is not a node section.
        if path.len() == 1 {
            return;
        }
        return;
    };
    if let Some((_, params)) = node.entry(ROS_PARAMETERS) {
        let mut section = ParamSection {
            node_key: path.join("/"),
            key_loc,
            params: Vec::new(),
        };
        flatten(file, "", params, &mut section.params);
        pf.sections.push(section);
        return;
    }
    if let Some((mk, _)) = entries
        .iter()
        .find(|(k, _)| k.as_str().is_some_and(is_misspelling))
    {
        pf.missing_ros_parameters.push(MissingRosParams {
            node_key: path.join("/"),
            key_loc,
            misspelled: Some((
                mk.as_str().unwrap_or_default().to_string(),
                Loc::new(file, mk.start(), mk.end()),
            )),
        });
        return;
    }
    if is_leafish(node) && !entries.is_empty() {
        pf.missing_ros_parameters.push(MissingRosParams {
            node_key: path.join("/"),
            key_loc,
            misspelled: None,
        });
        return;
    }
    for (k, v) in entries {
        if let Some(key) = k.as_str() {
            let mut p = path.clone();
            p.push(key.to_string());
            walk(file, p, Loc::new(file, k.start(), k.end()), v, pf);
        }
    }
}

fn flatten(file: FileId, prefix: &str, node: &YNode, out: &mut Vec<ParamEntry>) {
    for (k, v) in node.entries() {
        let Some(key) = k.as_str() else { continue };
        let name = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };
        if v.is_map() {
            flatten(file, &name, v, out);
        } else {
            out.push(ParamEntry {
                name,
                value: v.to_param_value(),
                key_loc: Loc::new(file, k.start(), k.end()),
                value_loc: Loc::new(file, v.start(), v.end()),
                raw: match v {
                    YNode::Scalar {
                        value,
                        quoted: false,
                        ..
                    } => Some(value.clone()),
                    _ => None,
                },
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::yaml;
    use crate::facts::ParamValue;

    fn pf(src: &str) -> ParamFile {
        extract(0, &yaml::parse(src).unwrap())
    }

    #[test]
    fn sections_and_flattening() {
        let p = pf("controller_server:\n  ros__parameters:\n    FollowPath:\n      max_vel: 0.5\n    freq: 20\n");
        assert_eq!(p.sections.len(), 1);
        let s = &p.sections[0];
        assert_eq!(s.node_key, "controller_server");
        let names: Vec<_> = s.params.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["FollowPath.max_vel", "freq"]);
        assert_eq!(s.params[1].value, ParamValue::Int(20));
    }

    #[test]
    fn misspelled_only_file_is_a_param_file() {
        assert!(looks_like_param_file(
            "driver:\n  ros__parametres:\n    x: 1\n"
        ));
        assert!(looks_like_param_file(
            "driver:\n  ros__paramters:\n    x: 1\n"
        ));
        assert!(looks_like_param_file(
            "driver:\n  ros_parameters:\n    x: 1\n"
        ));
        assert!(!looks_like_param_file(
            "jobs:\n  build:\n    parameters:\n      x: 1\n"
        ));
        assert!(!looks_like_param_file(
            "rosdistro: jazzy\nrepositories: []\n"
        ));
    }

    #[test]
    fn namespaced_keys() {
        let p = pf("robot1:\n  amcl:\n    ros__parameters:\n      a: 1\n/**:\n  ros__parameters:\n    use_sim_time: true\n");
        let keys: Vec<_> = p.sections.iter().map(|s| s.node_key.as_str()).collect();
        assert_eq!(keys, ["robot1/amcl", "/**"]);
    }

    #[test]
    fn missing_and_misspelled() {
        let p =
            pf("a:\n  x: 1\nb:\n  ros_parameters:\n    y: 2\nc:\n  ros__parameters:\n    z: 3\n");
        assert_eq!(p.sections.len(), 1);
        assert_eq!(p.missing_ros_parameters.len(), 2);
        assert!(p.missing_ros_parameters[0].misspelled.is_none());
        assert_eq!(
            p.missing_ros_parameters[1].misspelled.as_ref().unwrap().0,
            "ros_parameters"
        );
    }
}
