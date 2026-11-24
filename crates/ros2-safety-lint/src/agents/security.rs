//! Security agent: SROS2 governance, permissions, enclaves and policy-vs-graph.

use super::{Agent, Context};
use crate::diag::Diagnostic;
use crate::extract::sros2::parse_date_days;
use crate::facts::EndpointKind;
use crate::resolve::{expand_name, instance_endpoints, ImplStatus};
use crate::util::{closest, today_days};

pub struct SecurityAgent;

impl Agent for SecurityAgent {
    fn id(&self) -> &'static str {
        "security"
    }
    fn title(&self) -> &'static str {
        "SROS2 security configuration"
    }
    fn inputs(&self) -> &'static [crate::workspace::FileKind] {
        use crate::workspace::FileKind as K;
        // The launch graph plus SROS2 governance, permissions and policies.
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
            K::Security,
        ]
    }
    fn relevant(&self, cx: &Context) -> bool {
        let s = &cx.ix.security;
        !s.governance_weak.is_empty()
            || !s.grants.is_empty()
            || !s.policy_profiles.is_empty()
            || cx.graph.roots.iter().any(|r| {
                r.env.iter().any(|e| e.name.starts_with("ROS_SECURITY"))
                    || r.nodes.iter().any(|n| n.enclave.is_some())
            })
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        let s = &cx.ix.security;
        for g in &s.governance_weak {
            out.push(Diagnostic::new(
                "X001",
                g.loc,
                format!("governance sets `{}` to `{}`", g.key, g.value),
            ));
        }
        let today = today_days();
        for g in &s.grants {
            if let Some(loc) = g.default_allow {
                out.push(Diagnostic::new("X002", loc, format!("grant for `{}` allows everything not explicitly denied (`<default>ALLOW</default>`)", g.subject)));
            }
            for (t, loc) in &g.wildcard_rules {
                out.push(Diagnostic::new(
                    "X002",
                    *loc,
                    format!(
                        "grant for `{}` allows `{t}` (all topics/partitions)",
                        g.subject
                    ),
                ));
            }
            if let Some((d, loc)) = &g.not_after {
                if parse_date_days(d).is_some_and(|x| x < today) {
                    out.push(Diagnostic::new(
                        "X003",
                        *loc,
                        format!(
                            "permissions for `{}` expired on {}",
                            g.subject,
                            &d[..10.min(d.len())]
                        ),
                    ));
                }
            }
            if let Some((d, loc)) = &g.not_before {
                if parse_date_days(d).is_some_and(|x| x > today) {
                    out.push(Diagnostic::new(
                        "X003",
                        *loc,
                        format!(
                            "permissions for `{}` are not valid until {}",
                            g.subject,
                            &d[..10.min(d.len())]
                        ),
                    ));
                }
            }
        }
        let known: Vec<String> = s
            .enclaves
            .iter()
            .chain(cx.ix.keystore_enclaves.iter())
            .cloned()
            .collect();
        for root in &cx.graph.roots {
            for e in &root.env {
                if e.name == "ROS_SECURITY_KEYSTORE" {
                    if let Some(v) = &e.value {
                        let abs = v.starts_with('/')
                            || v.starts_with('~')
                            || v.get(1..3) == Some(":\\")
                            || v.get(1..3) == Some(":/");
                        if abs {
                            out.push(Diagnostic::new(
                                "X006",
                                e.loc,
                                format!("ROS_SECURITY_KEYSTORE is the absolute path `{v}`"),
                            ));
                        }
                    }
                }
            }
            for n in &root.nodes {
                if let (Some(enc), false) = (&n.enclave, known.is_empty()) {
                    if !known.iter().any(|k| k == enc) {
                        let mut d = Diagnostic::new("X004", n.decl.loc, format!("node `{}` uses enclave `{enc}`, which no policy or keystore in the workspace defines", n.display_name()));
                        if let Some(c) = closest(enc, known.iter().map(|s| s.as_str())) {
                            d = d.help(format!("Did you mean `{c}`?"));
                        }
                        if let Some(span) = super::value_span(cx.ix, n.decl.loc, enc) {
                            d = d.repair(span, enc.as_str(), known.clone());
                        }
                        out.push(d);
                    }
                }
                policy_vs_graph(cx, n, out);
            }
        }
    }
}

fn policy_vs_graph(cx: &Context, n: &crate::resolve::NodeInstance, out: &mut Vec<Diagnostic>) {
    if n.impl_status != ImplStatus::Resolved {
        return;
    }
    let Some(name) = &n.name else { return };
    let Some(prof) = cx
        .ix
        .security
        .policy_profiles
        .iter()
        .find(|p| p.node == *name && crate::resolve::normalize_ns(&p.ns) == n.namespace)
    else {
        return;
    };
    let (eps, complete) = instance_endpoints(cx.ix, n);
    if !complete {
        return;
    }
    let allowed = |list: &[String], topic: &str| {
        list.iter()
            .any(|t| t == "*" || expand_name(t, &prof.ns, &prof.node) == topic)
    };
    for e in eps {
        let (list, verb) = match e.kind {
            EndpointKind::Publisher => (&prof.publish, "publish"),
            EndpointKind::Subscription => (&prof.subscribe, "subscribe"),
            _ => continue,
        };
        if e.name.ends_with("/rosout") || e.name.ends_with("/parameter_events") {
            continue;
        }
        if !allowed(list, &e.name) {
            out.push(
                Diagnostic::new("X005", e.loc, format!("node `{}` will {verb} `{}`, but its policy (enclave `{}`) does not allow it", n.display_name(), e.name, prof.enclave))
                    .related(prof.loc, "policy profile"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::testutil::analyze_files;

    #[test]
    fn security_checks() {
        let d = analyze_files(&[
            ("p/package.xml", "<package format=\"3\"><name>p</name></package>"),
            ("p/sec/governance.xml", "<dds><domain_access_rules><domain_rule><allow_unauthenticated_participants>true</allow_unauthenticated_participants></domain_rule></domain_access_rules></dds>"),
            ("p/sec/permissions.xml", "<dds><permissions><grant name=\"g\"><subject_name>CN=/t</subject_name><validity><not_before>2020-01-01T00:00:00</not_before><not_after>2021-01-01T00:00:00</not_after></validity><allow_rule><publish><topics><topic>*</topic></topics></publish></allow_rule><default>DENY</default></grant></permissions></dds>"),
            ("p/sec/policy.xml", "<policy version=\"0.2.0\"><enclaves><enclave path=\"/demo/talker\"><profiles><profile ns=\"/\" node=\"talker\"><topics publish=\"ALLOW\"><topic>chatter</topic></topics></profile></profiles></enclave></enclaves></policy>"),
            ("p/launch/a.launch.xml", "<launch><node pkg=\"other\" exec=\"x\" ros_args=\"--enclave /demo/talkr\"/></launch>"),
        ]);
        let ids: Vec<&str> = d.iter().map(|x| x.rule).collect();
        for r in ["X001", "X002", "X003", "X004"] {
            assert!(ids.contains(&r), "{r}: {d:#?}");
        }
    }
}
