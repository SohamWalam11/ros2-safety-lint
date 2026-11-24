//! `rosfix policy`: an SROS2 access-control policy generated from the launch graph.
//!
//! `ros2 security generate_policy` records the graph of a running system, so it needs the
//! robot running and misses whatever did not happen during the recording. LiSA4ROS2 derives
//! policies statically, for Python nodes only. rosfix already has the launch graph with every
//! node's endpoints after namespaces and remaps, for C++ and Python, so it can write the policy
//! before anything runs.
//!
//! The output follows sros2's policy format 0.2.0 (`sros2/policy/schemas/policy.xsd`): one
//! enclave per node (its `--enclave` if set, else its fully qualified name) with one profile
//! that allows exactly what the node's code publishes, subscribes, serves, calls and executes,
//! plus the access every node needs (parameter services, `/parameter_events`, `rosout`,
//! `/clock`, type description), written inline as in sros2's `common/node.xml`. A node whose
//! interface rosfix could not fully determine gets an XML comment saying so instead of a guess.

use crate::engine::Analysis;
use crate::facts::EndpointKind;
use crate::resolve::{instance_endpoints, ImplStatus};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// What one node is allowed to do.
#[derive(Default)]
struct Profile {
    ns: String,
    node: String,
    publish: BTreeSet<String>,
    subscribe: BTreeSet<String>,
    reply: BTreeSet<String>,
    request: BTreeSet<String>,
    execute: BTreeSet<String>,
    call: BTreeSet<String>,
    notes: Vec<String>,
}

const COMMON: &str = "          <!-- every node: parameters, rosout, clock, type description (sros2 common/node.xml) -->
          <topics publish=\"ALLOW\" subscribe=\"ALLOW\">
            <topic>/parameter_events</topic>
          </topics>
          <topics publish=\"ALLOW\">
            <topic>rosout</topic>
          </topics>
          <topics subscribe=\"ALLOW\">
            <topic>/clock</topic>
          </topics>
          <services reply=\"ALLOW\" request=\"ALLOW\">
            <service>~/describe_parameters</service>
            <service>~/get_parameter_types</service>
            <service>~/get_parameters</service>
            <service>~/list_parameters</service>
            <service>~/set_parameters</service>
            <service>~/set_parameters_atomically</service>
            <service>~/get_type_description</service>
          </services>
";

/// The policy for the launch graphs (all, or the one of `launch`), and how many nodes were
/// left incomplete.
pub fn generate(a: &Analysis, launch: Option<&str>) -> Result<(String, usize), String> {
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
    // enclave path → (fully qualified node name → profile)
    let mut enclaves: BTreeMap<String, BTreeMap<String, Profile>> = BTreeMap::new();
    let mut skipped: Vec<String> = Vec::new();
    for r in &roots {
        for n in &r.nodes {
            let (Some(fq), Some(name)) = (n.fq_name.clone(), n.name.clone()) else {
                skipped.push(n.display_name());
                continue;
            };
            let enclave = n.enclave.clone().unwrap_or_else(|| fq.clone());
            let p = enclaves.entry(enclave).or_default().entry(fq.clone()).or_insert_with(|| Profile {
                ns: n.namespace.clone(),
                node: name,
                ..Default::default()
            });
            if n.impl_status != ImplStatus::Resolved {
                let why = "its source code was not found: only the access every node needs is allowed";
                if !p.notes.iter().any(|x| x == why) {
                    p.notes.push(why.into());
                }
                continue;
            }
            let (eps, complete) = instance_endpoints(&a.index, n);
            if !complete {
                let why = "some of its topics, services or actions could not be determined statically";
                if !p.notes.iter().any(|x| x == why) {
                    p.notes.push(why.into());
                }
            }
            for e in eps {
                let set = match e.kind {
                    EndpointKind::Publisher => &mut p.publish,
                    EndpointKind::Subscription => &mut p.subscribe,
                    EndpointKind::ServiceServer => &mut p.reply,
                    EndpointKind::ServiceClient => &mut p.request,
                    EndpointKind::ActionServer => &mut p.execute,
                    EndpointKind::ActionClient => &mut p.call,
                };
                set.insert(e.name);
            }
        }
    }
    if enclaves.is_empty() {
        return Err("no node with a known name in the launch graph".into());
    }
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        s,
        "<!-- Generated by rosfix {} from the launch graph{}. Review before use. -->",
        env!("CARGO_PKG_VERSION"),
        launch.map(|l| format!(" of {}", xml_escape(l))).unwrap_or_default()
    );
    skipped.sort();
    skipped.dedup();
    for k in &skipped {
        let _ = writeln!(s, "<!-- rosfix: node {} has no statically known name; it has no profile here -->", xml_escape(k));
    }
    s.push_str("<policy version=\"0.2.0\">\n  <enclaves>\n");
    let mut incomplete = 0;
    for (path, profiles) in &enclaves {
        let _ = writeln!(s, "    <enclave path=\"{}\">\n      <profiles>", xml_escape(path));
        for p in profiles.values() {
            let _ = writeln!(s, "        <profile ns=\"{}\" node=\"{}\">", xml_escape(&p.ns), xml_escape(&p.node));
            if !p.notes.is_empty() {
                incomplete += 1;
                for note in &p.notes {
                    let _ = writeln!(
                        s,
                        "          <!-- rosfix: {note}; complete this profile with `ros2 security generate_policy` on the running node -->"
                    );
                }
            }
            s.push_str(COMMON);
            let group = |s: &mut String, tag: &str, item: &str, attr: &str, names: &BTreeSet<String>| {
                if names.is_empty() {
                    return;
                }
                let _ = writeln!(s, "          <{tag} {attr}=\"ALLOW\">");
                for n in names {
                    let _ = writeln!(s, "            <{item}>{}</{item}>", xml_escape(n));
                }
                let _ = writeln!(s, "          </{tag}>");
            };
            group(&mut s, "topics", "topic", "publish", &p.publish);
            group(&mut s, "topics", "topic", "subscribe", &p.subscribe);
            group(&mut s, "services", "service", "reply", &p.reply);
            group(&mut s, "services", "service", "request", &p.request);
            group(&mut s, "actions", "action", "execute", &p.execute);
            group(&mut s, "actions", "action", "call", &p.call);
            s.push_str("        </profile>\n");
        }
        s.push_str("      </profiles>\n    </enclave>\n");
    }
    s.push_str("  </enclaves>\n</policy>\n");
    Ok((s, incomplete))
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::engine::{analyze, Options};

    #[test]
    fn talker_listener_policy() {
        let dir = tempfile::tempdir().unwrap();
        for (rel, text) in [
            ("p/package.xml", "<package format=\"3\"><name>p</name></package>"),
            ("p/CMakeLists.txt", "project(p)\nadd_executable(talker src/t.cpp)\nadd_executable(listener src/l.cpp)\ninstall(TARGETS talker listener DESTINATION lib/${PROJECT_NAME})\n"),
            ("p/src/t.cpp", "class T : public rclcpp::Node { public: T() : Node(\"talker\") { pub_ = create_publisher<std_msgs::msg::String>(\"chatter\", 10); srv_ = create_service<std_srvs::srv::Trigger>(\"~/reset\", cb); } };\n"),
            ("p/src/l.cpp", "class L : public rclcpp::Node { public: L() : Node(\"listener\") { sub_ = create_subscription<std_msgs::msg::String>(\"chatter\", 10, cb); } };\n"),
            ("p/launch/a.launch.xml", "<launch><node pkg=\"p\" exec=\"talker\" namespace=\"demo\"/><node pkg=\"p\" exec=\"listener\" namespace=\"demo\"><remap from=\"chatter\" to=\"/demo/chatter\"/></node><node pkg=\"other\" exec=\"x\" name=\"ext\"/></launch>"),
        ] {
            let f = dir.path().join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        }
        let a = analyze(dir.path(), &Config::default(), &Options::default());
        let (xml, incomplete) = super::generate(&a, None).unwrap();
        let doc = roxmltree::Document::parse(&xml).unwrap_or_else(|e| panic!("{e}\n{xml}"));
        assert_eq!(doc.root_element().attribute("version"), Some("0.2.0"));
        assert!(xml.contains("<enclave path=\"/demo/talker\">"), "{xml}");
        assert!(xml.contains("<profile ns=\"/demo\" node=\"talker\">"), "{xml}");
        assert!(xml.contains("<topics publish=\"ALLOW\">\n            <topic>/demo/chatter</topic>"), "{xml}");
        assert!(xml.contains("<topics subscribe=\"ALLOW\">\n            <topic>/demo/chatter</topic>"), "{xml}");
        assert!(xml.contains("<service>/demo/talker/reset</service>"), "{xml}");
        // The installed node without source: a profile with only the common access, flagged.
        assert_eq!(incomplete, 1, "{xml}");
        assert!(xml.contains("<profile ns=\"/\" node=\"ext\">\n          <!-- rosfix: its source code was not found"), "{xml}");
    }
}
