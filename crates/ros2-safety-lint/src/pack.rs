//! Knowledge packs: per-distro facts about popular ROS packages (declared parameters,
//! endpoints with QoS) extracted once from their source with rosfix's own extractors.
//!
//! Binary installs have no source code, so without a pack rosfix cannot check parameters
//! passed to e.g. Nav2 or ros2_controllers nodes. Packs are only used when the ROS
//! distribution is known, because parameter names differ between distributions.

use crate::facts::*;
use crate::resolve::link_impl;
use crate::workspace::{self, Index, IndexOptions};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;

pub const FORMAT: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Pack {
    pub format: u32,
    pub distro: String,
    /// (repository, branch/commit) the pack was built from.
    pub sources: Vec<(String, String)>,
    pub packages: BTreeMap<String, PackPackage>,
    /// pluginlib class names and types exported by the packages (plugin XML files): which
    /// `plugin:` values exist in this distribution.
    #[serde(default)]
    pub plugins: Vec<String>,
    /// Behavior-tree node IDs registered by the packages (`registerNodeType<…>("Id")`,
    /// `registerBuilder<…>("Id", …)`): which BT XML node types exist in this distribution.
    #[serde(default)]
    pub bt_nodes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PackPackage {
    pub executables: BTreeMap<String, PackNode>,
    /// rclcpp components (`ns::Class`).
    pub components: BTreeMap<String, PackNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PackNode {
    pub default_name: Option<String>,
    pub params: Vec<PackParam>,
    pub params_open: bool,
    pub params_open_dotted: bool,
    pub endpoints: Vec<PackEndpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackParam {
    pub name: NamePattern,
    pub ty: ParamType,
    pub has_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackEndpoint {
    pub kind: EndpointKind,
    pub name: String,
    pub msg_type: Option<String>,
    pub qos: QosSpec,
    pub qos_overrides_enabled: bool,
}

// ---------------------------------------------------------------------------- embedded packs

const HUMBLE: &[u8] = include_bytes!("../packs/humble.json.gz");
const JAZZY: &[u8] = include_bytes!("../packs/jazzy.json.gz");
const KILTED: &[u8] = include_bytes!("../packs/kilted.json.gz");

pub fn decode(bytes: &[u8]) -> Option<Pack> {
    if bytes.is_empty() {
        return None;
    }
    let mut s = String::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_string(&mut s)
        .ok()?;
    let p: Pack = serde_json::from_str(&s).ok()?;
    (p.format == FORMAT).then_some(p)
}

/// The embedded pack for a distribution (Rolling uses the newest).
pub fn embedded(distro: &str) -> Option<Pack> {
    let bytes = match distro {
        "humble" => HUMBLE,
        "jazzy" => JAZZY,
        "kilted" | "rolling" => KILTED,
        _ => return None,
    };
    decode(bytes)
}

// ---------------------------------------------------------------------------- rosdep keys

const KEYS_HUMBLE: &[u8] = include_bytes!("../packs/keys/humble.txt.gz");
const KEYS_JAZZY: &[u8] = include_bytes!("../packs/keys/jazzy.txt.gz");
const KEYS_KILTED: &[u8] = include_bytes!("../packs/keys/kilted.txt.gz");

/// Names `rosdep install` can resolve for a distribution: released packages and system
/// dependency keys (built by `scripts/build_rosdep_keys.py`). Decoded once per distribution.
pub fn rosdep_keys(distro: &str) -> Option<&'static std::collections::HashSet<String>> {
    use std::collections::HashSet;
    use std::sync::OnceLock;
    static CACHE: [OnceLock<Option<HashSet<String>>>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    let (i, bytes) = match distro {
        "humble" => (0, KEYS_HUMBLE),
        "jazzy" => (1, KEYS_JAZZY),
        "kilted" | "rolling" => (2, KEYS_KILTED),
        _ => return None,
    };
    CACHE[i]
        .get_or_init(|| {
            let mut s = String::new();
            flate2::read::GzDecoder::new(bytes).read_to_string(&mut s).ok()?;
            Some(s.lines().map(str::to_string).collect())
        })
        .as_ref()
}

/// The embedded pack of a distribution, decoded once (shared by checks that compare
/// distributions).
pub fn cached(distro: &str) -> Option<&'static Pack> {
    use std::sync::OnceLock;
    static CACHE: [OnceLock<Option<Pack>>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    let i = match distro {
        "humble" => 0,
        "jazzy" => 1,
        "kilted" | "rolling" => 2,
        _ => return None,
    };
    CACHE[i].get_or_init(|| embedded(distro)).as_ref()
}

pub fn encode(p: &Pack) -> Vec<u8> {
    use std::io::Write;
    let json = serde_json::to_vec(p).expect("serializable");
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(&json).expect("in-memory write");
    e.finish().expect("in-memory gzip")
}

// ---------------------------------------------------------------------------- building

fn pack_node(ix: &Index, linked: &crate::resolve::Linked) -> Option<PackNode> {
    let (impls, _, spec, open, open_dotted) = linked;
    if impls.is_empty() {
        return None;
    }
    let mut node = PackNode {
        params_open: *open,
        params_open_dotted: *open_dotted,
        ..Default::default()
    };
    let names: Vec<&str> = impls
        .iter()
        .filter_map(|(f, i)| ix.code[f].nodes[*i].default_name.as_deref())
        .collect();
    if names.len() == 1 {
        node.default_name = Some(names[0].to_string());
    }
    let mut seen = std::collections::HashSet::new();
    let decls = impls
        .iter()
        .flat_map(|(f, i)| ix.code[f].nodes[*i].params.iter())
        .chain(spec.iter());
    for d in decls {
        if seen.insert(d.name.clone()) {
            node.params.push(PackParam {
                name: d.name.clone(),
                ty: d.ty,
                has_default: d.has_default,
            });
        }
    }
    for (f, i) in impls {
        for e in &ix.code[f].nodes[*i].endpoints {
            if let Some(name) = &e.name {
                node.endpoints.push(PackEndpoint {
                    kind: e.kind,
                    name: name.clone(),
                    msg_type: e.msg_type.clone(),
                    qos: e.qos.clone(),
                    qos_overrides_enabled: e.qos_overrides_enabled,
                });
            }
        }
    }
    Some(node)
}

/// Builds a pack from source checkouts (each a directory of ROS packages).
pub fn build(distro: &str, roots: &[(PathBuf, String, String)]) -> Pack {
    let config = crate::config::Config::default();
    let mut pack = Pack {
        format: FORMAT,
        distro: distro.to_string(),
        ..Default::default()
    };
    for (root, repo, rev) in roots {
        pack.sources.push((repo.clone(), rev.clone()));
        let ix = workspace::index(root, &config, &IndexOptions::default());
        pack.plugins.extend(ix.packages.iter().flat_map(|p| p.plugin_classes.iter().cloned()));
        for f in ix.code.keys() {
            let rel = &ix.file(*f).rel_path;
            if !rel.split('/').any(|seg| seg == "test" || seg == "tests") {
                pack.bt_nodes.extend(bt_registrations(&ix.file(*f).text));
            }
        }
        for p in &ix.packages {
            let mut pp = PackPackage::default();
            for x in &p.executables {
                if x.sources.iter().any(|s| config.is_test_path(s)) || !x.installed {
                    continue;
                }
                let linked = link_impl(
                    &ix,
                    Some(&p.name),
                    Some(&x.name),
                    None,
                    LaunchNodeKind::Node,
                );
                if let Some(n) = pack_node(&ix, &linked) {
                    pp.executables.insert(x.name.clone(), n);
                }
            }
            for cls in p.plugin_classes.iter().filter(|c| c.contains("::")) {
                let linked = link_impl(
                    &ix,
                    Some(&p.name),
                    None,
                    Some(cls),
                    LaunchNodeKind::Composable,
                );
                if let Some(n) = pack_node(&ix, &linked) {
                    pp.components.insert(cls.clone(), n);
                }
            }
            if !pp.executables.is_empty() || !pp.components.is_empty() {
                pack.packages.insert(p.name.clone(), pp);
            }
        }
    }
    pack.plugins.sort();
    pack.plugins.dedup();
    pack.bt_nodes.sort();
    pack.bt_nodes.dedup();
    pack
}

/// Behavior-tree node IDs registered in C++ source: `registerNodeType<T>("Id")`,
/// `registerBuilder<T>("Id", …)`, `registerSimpleAction("Id", …)` and similar.
pub fn bt_registrations(src: &str) -> Vec<String> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r#"register(?:NodeType|Builder|SimpleAction|SimpleCondition|SimpleDecorator)\s*(?:<[^;()]*?>)?\s*\(\s*"([A-Za-z_][\w]*)""#).expect("valid regex")
    });
    re.captures_iter(src).map(|c| c[1].to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut p = Pack {
            format: FORMAT,
            distro: "jazzy".into(),
            ..Default::default()
        };
        p.packages.insert("x".into(), PackPackage::default());
        let back = decode(&encode(&p)).unwrap();
        assert_eq!(back.distro, "jazzy");
        assert!(back.packages.contains_key("x"));
    }
}
