//! DDS-Security governance/permissions documents and SROS2 policy files.

use crate::facts::{GovernanceRule, PermissionGrant, PolicyProfile, SecurityFacts};
use crate::source::{FileId, Loc};

pub fn is_relevant(text: &str) -> bool {
    text.contains("<domain_access_rules")
        || text.contains("<permissions")
        || (text.contains("<policy") && text.contains("<enclave"))
}

pub fn extract(file: FileId, text: &str, out: &mut SecurityFacts) -> Result<(), String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    let loc = |n: &roxmltree::Node| Loc::new(file, n.range().start, n.range().end);
    // ------------------------------------------------------------- governance
    for rule in doc
        .descendants()
        .filter(|n| n.has_tag_name("domain_rule") || n.has_tag_name("topic_rule"))
    {
        for c in rule.children().filter(|c| c.is_element()) {
            let key = c.tag_name().name();
            let val = c.text().unwrap_or("").trim().to_string();
            let weak = match key {
                "allow_unauthenticated_participants" => val.eq_ignore_ascii_case("true"),
                "enable_join_access_control"
                | "enable_read_access_control"
                | "enable_write_access_control" => val.eq_ignore_ascii_case("false"),
                k if k.ends_with("_protection_kind") => val.eq_ignore_ascii_case("NONE"),
                _ => false,
            };
            if weak {
                let scope = if rule.has_tag_name("topic_rule") {
                    let expr = rule
                        .children()
                        .find(|x| x.has_tag_name("topic_expression"))
                        .and_then(|x| x.text())
                        .unwrap_or("?");
                    format!("{key} (topics `{}`)", expr.trim())
                } else {
                    key.to_string()
                };
                out.governance_weak.push(GovernanceRule {
                    key: scope,
                    value: val,
                    loc: loc(&c),
                });
            }
        }
    }
    // ------------------------------------------------------------- permissions
    for g in doc.descendants().filter(|n| n.has_tag_name("grant")) {
        let subject = g
            .children()
            .find(|c| c.has_tag_name("subject_name"))
            .and_then(|c| c.text())
            .unwrap_or("")
            .trim()
            .to_string();
        let mut grant = PermissionGrant {
            subject,
            default_allow: None,
            wildcard_rules: Vec::new(),
            not_before: None,
            not_after: None,
            loc: loc(&g),
        };
        for c in g.descendants().filter(|c| c.is_element()) {
            match c.tag_name().name() {
                "default"
                    if c.text()
                        .is_some_and(|t| t.trim().eq_ignore_ascii_case("ALLOW")) =>
                {
                    grant.default_allow = Some(loc(&c))
                }
                "not_before" => {
                    grant.not_before = c.text().map(|t| (t.trim().to_string(), loc(&c)))
                }
                "not_after" => grant.not_after = c.text().map(|t| (t.trim().to_string(), loc(&c))),
                "topic" | "partition" => {
                    let t = c.text().unwrap_or("").trim();
                    let in_allow = c.ancestors().any(|a| a.has_tag_name("allow_rule"));
                    if in_allow && matches!(t, "*" | "rt/*" | "rq/*" | "rr/*" | "*/*") {
                        grant.wildcard_rules.push((t.to_string(), loc(&c)));
                    }
                }
                _ => {}
            }
        }
        out.grants.push(grant);
    }
    // ------------------------------------------------------------- SROS2 policy
    for e in doc.descendants().filter(|n| n.has_tag_name("enclave")) {
        let path = e.attribute("path").unwrap_or("").to_string();
        if !path.is_empty() && !out.enclaves.contains(&path) {
            out.enclaves.push(path.clone());
        }
        for p in e.descendants().filter(|n| n.has_tag_name("profile")) {
            let mut prof = PolicyProfile {
                enclave: path.clone(),
                ns: p.attribute("ns").unwrap_or("/").to_string(),
                node: p.attribute("node").unwrap_or("").to_string(),
                publish: Vec::new(),
                subscribe: Vec::new(),
                loc: loc(&p),
            };
            for t in p.children().filter(|c| c.has_tag_name("topics")) {
                let names: Vec<String> = t
                    .children()
                    .filter(|c| c.has_tag_name("topic"))
                    .filter_map(|c| c.text())
                    .map(|s| s.trim().to_string())
                    .collect();
                if t.attribute("publish")
                    .is_some_and(|v| v.eq_ignore_ascii_case("ALLOW"))
                {
                    prof.publish.extend(names.iter().cloned());
                }
                if t.attribute("subscribe")
                    .is_some_and(|v| v.eq_ignore_ascii_case("ALLOW"))
                {
                    prof.subscribe.extend(names.iter().cloned());
                }
            }
            out.policy_profiles.push(prof);
        }
    }
    Ok(())
}

/// Parses `YYYY-MM-DDTHH:MM:SS` → days since epoch.
pub fn parse_date_days(s: &str) -> Option<i64> {
    let d = s.get(0..10)?;
    let mut it = d.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let day: u32 = it.next()?.parse().ok()?;
    Some(crate::util::days_from_civil(y, m, day))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn governance_and_permissions() {
        let gov = r#"<dds><domain_access_rules><domain_rule><allow_unauthenticated_participants>true</allow_unauthenticated_participants><rtps_protection_kind>ENCRYPT</rtps_protection_kind><topic_access_rules><topic_rule><topic_expression>*</topic_expression><data_protection_kind>NONE</data_protection_kind></topic_rule></topic_access_rules></domain_rule></domain_access_rules></dds>"#;
        let mut f = SecurityFacts::default();
        extract(0, gov, &mut f).unwrap();
        assert_eq!(f.governance_weak.len(), 2);
        let perm = r#"<dds><permissions><grant name="g"><subject_name>CN=/talker</subject_name><validity><not_before>2020-01-01T00:00:00</not_before><not_after>2021-01-01T00:00:00</not_after></validity><allow_rule><publish><topics><topic>*</topic></topics></publish></allow_rule><default>ALLOW</default></grant></permissions></dds>"#;
        extract(0, perm, &mut f).unwrap();
        let g = &f.grants[0];
        assert!(g.default_allow.is_some());
        assert_eq!(g.wildcard_rules.len(), 1);
        assert_eq!(
            parse_date_days(&g.not_after.as_ref().unwrap().0),
            Some(crate::util::days_from_civil(2021, 1, 1))
        );
    }

    #[test]
    fn policy() {
        let p = r#"<policy version="0.2.0"><enclaves><enclave path="/tl/talker"><profiles><profile ns="/" node="talker"><topics publish="ALLOW"><topic>chatter</topic></topics></profile></profiles></enclave></enclaves></policy>"#;
        let mut f = SecurityFacts::default();
        extract(0, p, &mut f).unwrap();
        assert_eq!(f.enclaves, vec!["/tl/talker"]);
        assert_eq!(f.policy_profiles[0].publish, vec!["chatter"]);
    }
}
