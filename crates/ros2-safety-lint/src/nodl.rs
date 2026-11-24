//! NoDL (Node Definition Language, ros-tooling/nodl, format version 2): node interfaces
//! declared in YAML.
//!
//! In: installed packages register NoDL documents in the ament index
//! (`share/ament_index/resource_index/nodl/<package>__<name>`). A document named after an
//! executable is used like a knowledge-pack entry for it: its parameters and endpoints are
//! checked against the launch files, exactly as for nodes whose source is available.
//! Knowledge-pack entries win over NoDL documents for the same executable; documents with
//! `include:` references (inherited interfaces) mark the parameters open, so nothing is
//! reported as undeclared that an included interface might declare.
//!
//! Out: `rosfix nodl export` writes a NoDL document for each workspace executable from what
//! rosfix extracted from its source. Only facts read from the code are written; anything that
//! could not be determined statically (a computed default, an unknown QoS) is written as a
//! comment starting with `# ?`, so the document never claims more than the code shows.

use crate::engine::Analysis;
use crate::extract::yaml::{self, YNode};
use crate::facts::*;
use crate::pack::{Pack, PackEndpoint, PackNode, PackParam};
use crate::resolve::{link_impl, ImplStatus};
use crate::workspace::Index;
use std::fmt::Write;

// ---------------------------------------------------------------------------- import

fn param_type(t: &str) -> ParamType {
    let base = t.split("_fixed_").next().unwrap_or(t);
    match base {
        "bool" => ParamType::Bool,
        "int" => ParamType::Int,
        "double" => ParamType::Double,
        "string" => ParamType::Str,
        "bool_array" => ParamType::BoolArray,
        "int_array" => ParamType::IntArray,
        "double_array" => ParamType::DoubleArray,
        "string_array" => ParamType::StrArray,
        "byte_array" => ParamType::ByteArray,
        _ => ParamType::Unknown,
    }
}

/// `pkg/msg/Type` (NoDL) → `pkg::msg::Type` (how rosfix records types from C++).
fn ros_type(t: &str) -> String {
    t.trim().replace('/', "::")
}

fn qos(q: Option<&YNode>, origin: &str) -> QosSpec {
    let mut s = QosSpec::unknown(origin);
    let Some(q) = q else { return s };
    let get = |k: &str| q.get(k).and_then(YNode::as_str).map(str::trim);
    s.reliability = match get("reliability") {
        Some("RELIABLE") => Some(Reliability::Reliable),
        Some("BEST_EFFORT") => Some(Reliability::BestEffort),
        _ => None,
    };
    s.durability = match get("durability") {
        Some("VOLATILE") => Some(Durability::Volatile),
        Some("TRANSIENT_LOCAL") => Some(Durability::TransientLocal),
        _ => None,
    };
    s.liveliness = match get("liveliness") {
        Some("AUTOMATIC") => Some(LivelinessKind::Automatic),
        Some("MANUAL_BY_TOPIC") => Some(LivelinessKind::ManualByTopic),
        _ => None,
    };
    if get("history") == Some("KEEP_LAST") {
        s.depth = get("depth").and_then(|d| d.parse().ok());
    }
    s.deadline_ns = get("deadline_ns").and_then(|d| d.parse().ok());
    s.lease_ns = get("liveliness_lease_duration_ns").and_then(|d| d.parse().ok());
    s.system_default = get("reliability") == Some("SYSTEM_DEFAULT");
    s
}

/// A NoDL document as a knowledge-pack node. None when the text is not a NoDL v2 document.
pub fn to_pack_node(text: &str, origin: &str) -> Option<PackNode> {
    let docs = yaml::parse(text).ok()?;
    let doc = docs.into_iter().next()?;
    if doc
        .get("nodl_version")
        .and_then(YNode::as_str)
        .map(str::trim)
        != Some("2")
    {
        return None;
    }
    let mut n = PackNode {
        params_open: doc.get("include").is_some(),
        ..Default::default()
    };
    if let Some(p) = doc.get("parameters") {
        for (k, v) in p.entries() {
            let Some(name) = k.as_str() else { continue };
            let ty = v
                .get("type")
                .and_then(YNode::as_str)
                .map_or(ParamType::Unknown, param_type);
            n.params.push(PackParam {
                name: NamePattern::Exact(name.to_string()),
                ty,
                has_default: v.get("default_value").is_some(),
            });
        }
    }
    let kinds = [
        ("publishers", EndpointKind::Publisher),
        ("subscriptions", EndpointKind::Subscription),
        ("service_servers", EndpointKind::ServiceServer),
        ("service_clients", EndpointKind::ServiceClient),
        ("action_servers", EndpointKind::ActionServer),
        ("action_clients", EndpointKind::ActionClient),
    ];
    for (key, kind) in kinds {
        for e in doc.get(key).map(YNode::items).unwrap_or_default() {
            let Some(name) = e.get("name").and_then(YNode::as_str) else {
                continue;
            };
            n.endpoints.push(PackEndpoint {
                kind,
                name: name.to_string(),
                msg_type: e.get("type").and_then(YNode::as_str).map(ros_type),
                qos: qos(e.get("qos"), &format!("NoDL {origin}")),
                qos_overrides_enabled: false,
            });
        }
    }
    Some(n)
}

/// Adds the NoDL documents that installed packages register (`<pkg>__<executable>`) to the
/// knowledge pack, for executables the pack does not already describe.
pub fn merge_installed(ix: &mut Index, wanted: &std::collections::HashSet<String>) {
    for pkg in ix.installed.packages.values() {
        if !wanted.contains(&pkg.name) || ix.package_by_name(&pkg.name).is_some() {
            continue;
        }
        let dir = pkg.prefix.join("share/ament_index/resource_index/nodl");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let prefix = format!("{}__", pkg.name);
        for e in entries.flatten() {
            let file = e.file_name().to_string_lossy().to_string();
            let Some(exe) = file.strip_prefix(&prefix) else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(e.path()) else {
                continue;
            };
            let Some(node) = to_pack_node(&text, &format!("{}/{exe}", pkg.name)) else {
                continue;
            };
            let pack = ix.pack.get_or_insert_with(|| Pack {
                format: crate::pack::FORMAT,
                distro: "nodl".into(),
                ..Default::default()
            });
            pack.packages
                .entry(pkg.name.clone())
                .or_default()
                .executables
                .entry(exe.to_string())
                .or_insert(node);
        }
    }
}

// ---------------------------------------------------------------------------- export

pub struct Exported {
    pub package: String,
    pub executable: String,
    pub text: String,
}

fn type_name(t: ParamType) -> Option<&'static str> {
    Some(match t {
        ParamType::Bool => "bool",
        ParamType::Int => "int",
        ParamType::Double => "double",
        ParamType::Str => "string",
        ParamType::BoolArray => "bool_array",
        ParamType::IntArray => "int_array",
        ParamType::DoubleArray => "double_array",
        ParamType::StrArray => "string_array",
        ParamType::ByteArray => "byte_array",
        ParamType::Unknown => return None,
    })
}

/// A double-quoted YAML string.
fn quoted(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| format!("{s:?}"))
}

/// A C++ or Python literal default as a YAML value; None when it is not a plain literal.
pub fn literal_yaml(text: &str) -> Option<String> {
    let t = text.trim();
    // std::string("x"), std::vector<double>{1, 2}, rclcpp::ParameterValue(1.0)
    for wrapper in ["std::string(", "rclcpp::ParameterValue(", "ParameterValue("] {
        if let Some(inner) = t.strip_prefix(wrapper).and_then(|r| r.strip_suffix(')')) {
            return literal_yaml(inner);
        }
    }
    if let Some(i) = t.find('{').filter(|_| t.starts_with("std::vector<")) {
        return literal_yaml(&t[i..]);
    }
    if let Some(body) = t
        .strip_prefix('{')
        .and_then(|r| r.strip_suffix('}'))
        .or_else(|| t.strip_prefix('[').and_then(|r| r.strip_suffix(']')))
    {
        if body.trim().is_empty() {
            return Some("[]".into());
        }
        let items: Option<Vec<String>> = split_top(body).iter().map(|s| literal_yaml(s)).collect();
        return items.map(|v| format!("[{}]", v.join(", ")));
    }
    match t {
        "true" | "True" => return Some("true".into()),
        "false" | "False" => return Some("false".into()),
        _ => {}
    }
    if (t.starts_with('"') && t.ends_with('"') && t.len() >= 2)
        || (t.starts_with('\'') && t.ends_with('\'') && t.len() >= 2)
    {
        let inner = &t[1..t.len() - 1];
        if inner.contains('\\') {
            return None; // escape sequences: not worth guessing
        }
        return Some(quoted(inner));
    }
    let num = t.trim_end_matches(['f', 'F', 'L', 'l', 'u', 'U']);
    if num.parse::<i64>().is_ok() {
        return Some(num.to_string());
    }
    if num.parse::<f64>().is_ok_and(f64::is_finite) {
        // YAML reads `1.` or `.5` fine, but write the canonical form.
        let v: f64 = num.parse().ok()?;
        return Some(if v.fract() == 0.0 && !num.contains(['e', 'E']) {
            format!("{v:.1}")
        } else {
            format!("{v}")
        });
    }
    None
}

/// Splits a literal list on top-level commas.
fn split_top(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth, mut quote) = (Vec::new(), String::new(), 0i32, None);
    for c in s.chars() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '{' | '[' | '(') => depth += 1,
            (None, '}' | ']' | ')') => depth -= 1,
            (None, ',') if depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

fn qos_yaml(q: &QosSpec) -> Option<String> {
    let reliability = match (q.reliability, q.system_default) {
        (Some(Reliability::Reliable), _) => "RELIABLE",
        (Some(Reliability::BestEffort), _) => "BEST_EFFORT",
        (None, true) => "SYSTEM_DEFAULT",
        (None, false) => return None,
    };
    let mut parts = Vec::new();
    match q.depth {
        Some(d) => parts.push(format!("history: KEEP_LAST, depth: {d}")),
        None if q.system_default => parts.push("history: SYSTEM_DEFAULT".into()),
        None => return None,
    }
    parts.push(format!("reliability: {reliability}"));
    match q.durability {
        Some(Durability::Volatile) => parts.push("durability: VOLATILE".into()),
        Some(Durability::TransientLocal) => parts.push("durability: TRANSIENT_LOCAL".into()),
        None if q.system_default => parts.push("durability: SYSTEM_DEFAULT".into()),
        None => {}
    }
    Some(format!("{{{}}}", parts.join(", ")))
}

/// One NoDL document per workspace executable whose source rosfix can link.
pub fn export(a: &Analysis) -> Vec<Exported> {
    let ix = &a.index;
    let mut out = Vec::new();
    for p in &ix.packages {
        let mut exes: Vec<&str> = p.executables.iter().map(|e| e.name.as_str()).collect();
        exes.sort();
        exes.dedup();
        for exe in exes {
            let (impls, status, spec_params, open, open_dotted) =
                link_impl(ix, Some(&p.name), Some(exe), None, LaunchNodeKind::Node);
            if status != ImplStatus::Resolved || impls.is_empty() {
                continue;
            }
            let nodes: Vec<&NodeImpl> = impls.iter().map(|(f, i)| &ix.code[f].nodes[*i]).collect();
            out.push(Exported {
                package: p.name.clone(),
                executable: exe.to_string(),
                text: document(ix, &nodes, &spec_params, open, open_dotted),
            });
        }
    }
    out
}

fn document(
    ix: &Index,
    nodes: &[&NodeImpl],
    spec_params: &[ParamDecl],
    open: bool,
    open_dotted: bool,
) -> String {
    let mut s = String::from("---\n");
    let sources: Vec<String> = nodes
        .iter()
        .map(|n| match &n.class_name {
            Some(c) => format!("{} (class {c})", ix.file(n.file).rel_path),
            None => ix.file(n.file).rel_path.clone(),
        })
        .collect();
    let _ = writeln!(
        s,
        "# Generated by rosfix {} from {}.",
        env!("CARGO_PKG_VERSION"),
        sources.join(", ")
    );
    let _ = writeln!(
        s,
        "# Review before use. Lines starting with \"# ?\" could not be determined from the code."
    );
    if open {
        s.push_str("# ? the node also accepts parameters rosfix cannot enumerate (dynamic names or undeclared parameters)\n");
    } else if open_dotted {
        s.push_str(
            "# ? the node also accepts plugin parameters (dotted names) rosfix cannot enumerate\n",
        );
    }
    s.push_str("nodl_version: 2\n");

    // Parameters: declarations from code, then generate_parameter_library specs.
    let mut params: Vec<&ParamDecl> = nodes
        .iter()
        .flat_map(|n| n.params.iter())
        .chain(spec_params.iter())
        .collect();
    params.sort_by(|a, b| a.name.literal().cmp(b.name.literal()));
    params.dedup_by(|a, b| a.name == b.name);
    if !params.is_empty() {
        let mut sec = Section::default();
        for p in params {
            let NamePattern::Exact(name) = &p.name else {
                sec.note(format!(
                    "parameters ending in `{}` under a prefix known only at runtime",
                    p.name.literal()
                ));
                continue;
            };
            let Some(ty) = type_name(p.ty) else {
                sec.note(format!("{name}: type not known statically"));
                continue;
            };
            let default = p.default_text.as_deref().and_then(literal_yaml);
            match (p.has_default, default) {
                (true, Some(v)) => sec.entry(format!(
                    "  {}:\n    type: {ty}\n    default_value: {v}\n",
                    yaml_key(name)
                )),
                (true, None) => {
                    let shown = p.default_text.as_deref().unwrap_or("(computed)");
                    sec.note(format!(
                        "{name}: {ty} with a default set in code that is not a literal: {shown}"
                    ));
                }
                (false, _) => sec.entry(format!("  {}:\n    type: {ty}\n", yaml_key(name))),
            }
        }
        sec.write(&mut s, "parameters");
    }

    let groups = [
        ("publishers", EndpointKind::Publisher, true),
        ("subscriptions", EndpointKind::Subscription, true),
        ("service_servers", EndpointKind::ServiceServer, false),
        ("service_clients", EndpointKind::ServiceClient, false),
        ("action_servers", EndpointKind::ActionServer, false),
        ("action_clients", EndpointKind::ActionClient, false),
    ];
    for (key, kind, needs_qos) in groups {
        let mut eps: Vec<&Endpoint> = nodes
            .iter()
            .flat_map(|n| n.endpoints.iter())
            .filter(|e| e.kind == kind)
            .collect();
        eps.sort_by(|a, b| a.name.cmp(&b.name));
        eps.dedup_by(|a, b| a.name == b.name && a.msg_type == b.msg_type);
        if eps.is_empty() {
            continue;
        }
        let mut sec = Section::default();
        for e in eps {
            let Some(name) = &e.name else {
                sec.note("an endpoint whose name is computed at runtime".into());
                continue;
            };
            // A full interface name (`pkg/msg/Type`); Python nodes often record only the class.
            let Some(ty) = e
                .msg_type
                .as_deref()
                .map(|t| t.trim().replace("::", "/"))
                .filter(|t| t.split('/').count() >= 2)
            else {
                sec.note(format!("{name}: message type not known statically"));
                continue;
            };
            let q = if needs_qos { qos_yaml(&e.qos) } else { None };
            match (needs_qos, q) {
                (true, None) => sec.note(format!(
                    "{name} ({ty}): QoS not known statically ({})",
                    e.qos.origin
                )),
                (true, Some(q)) => sec.entry(format!(
                    "  - name: {}\n    type: {ty}\n    qos: {q}\n",
                    quoted(name)
                )),
                (false, _) => sec.entry(format!("  - name: {}\n    type: {ty}\n", quoted(name))),
            }
        }
        sec.write(&mut s, key);
    }
    s
}

/// One section of a document: its entries, and notes about what could not be exported.
/// A section with notes only is written as comments without the key: an empty key would be
/// `null`, which the schema rejects, and `[]` would claim the node has none.
#[derive(Default)]
struct Section {
    entries: String,
    notes: Vec<String>,
}

impl Section {
    fn entry(&mut self, text: String) {
        self.entries.push_str(&text);
    }

    fn note(&mut self, text: String) {
        // One comment line: source text may contain newlines (and CRLF).
        self.notes
            .push(text.split_whitespace().collect::<Vec<_>>().join(" "));
    }

    fn write(&self, s: &mut String, key: &str) {
        if !self.entries.is_empty() {
            let _ = writeln!(s, "{key}:");
            s.push_str(&self.entries);
        }
        for n in &self.notes {
            let _ = writeln!(s, "# ? {key}: {n}");
        }
    }
}

/// A mapping key, quoted when YAML would not read it as the same plain string.
fn yaml_key(k: &str) -> String {
    let plain = !k.is_empty()
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-'))
        && !matches!(k, "true" | "false" | "null" | "yes" | "no" | "on" | "off");
    if plain {
        k.to_string()
    } else {
        quoted(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals() {
        assert_eq!(literal_yaml("1.0").as_deref(), Some("1.0"));
        assert_eq!(literal_yaml("0.5f").as_deref(), Some("0.5"));
        assert_eq!(literal_yaml("10").as_deref(), Some("10"));
        assert_eq!(literal_yaml("-3").as_deref(), Some("-3"));
        assert_eq!(literal_yaml("1e-3").as_deref(), Some("0.001"));
        assert_eq!(literal_yaml("true").as_deref(), Some("true"));
        assert_eq!(literal_yaml("\"map\"").as_deref(), Some("\"map\""));
        assert_eq!(
            literal_yaml("std::string(\"base\")").as_deref(),
            Some("\"base\"")
        );
        assert_eq!(
            literal_yaml("std::vector<double>{1.0, 2}").as_deref(),
            Some("[1.0, 2]")
        );
        assert_eq!(
            literal_yaml("{\"a\", \"b,c\"}").as_deref(),
            Some("[\"a\", \"b,c\"]")
        );
        assert_eq!(literal_yaml("[1, 2]").as_deref(), Some("[1, 2]"));
        assert_eq!(literal_yaml("kDefault"), None);
        assert_eq!(literal_yaml("M_PI / 2"), None);
    }

    #[test]
    fn imported_document_becomes_a_pack_node() {
        let doc = "nodl_version: 2\nparameters:\n  rate:\n    type: double\n    default_value: 1.0\n  cells:\n    type: int\npublishers:\n  - name: ~/state\n    type: sensor_msgs/msg/BatteryState\n    qos: {history: KEEP_LAST, depth: 5, reliability: RELIABLE}\nservice_servers:\n  - name: ~/reset\n    type: std_srvs/srv/Trigger\n";
        let n = to_pack_node(doc, "p/x").unwrap();
        assert!(!n.params_open);
        assert_eq!(n.params.len(), 2);
        assert!(n
            .params
            .iter()
            .any(|p| p.name == NamePattern::Exact("rate".into())
                && p.ty == ParamType::Double
                && p.has_default));
        assert!(n
            .params
            .iter()
            .any(|p| p.name == NamePattern::Exact("cells".into()) && !p.has_default));
        let pubs: Vec<_> = n
            .endpoints
            .iter()
            .filter(|e| e.kind == EndpointKind::Publisher)
            .collect();
        assert_eq!(
            pubs[0].msg_type.as_deref(),
            Some("sensor_msgs::msg::BatteryState")
        );
        assert_eq!(pubs[0].qos.reliability, Some(Reliability::Reliable));
        assert_eq!(pubs[0].qos.depth, Some(5));
        assert!(to_pack_node("nodl_version: 1\n", "p/x").is_none());
        assert!(
            to_pack_node(
                "nodl_version: 2\ninclude:\n  - ref: nodl://nodl_common_interfaces/node\n",
                "p/x"
            )
            .unwrap()
            .params_open
        );
    }
}
