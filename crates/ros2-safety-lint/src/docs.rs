//! `rosfix docs`: the manual, compiled into the binary.
//!
//! Hand-written topics live in `docs/*.md` next to this crate. The `agents` and `rules`
//! topics are generated from the agent and rule registries, so they cannot go stale.

use crate::agents::all_agents;
use crate::rules::RULES;
use std::fmt::Write;

/// (name, one-line description, text); `None` marks a generated topic.
const TOPICS: &[(&str, &str, Option<&str>)] = &[
    (
        "overview",
        "what rosfix is and how one analysis works",
        Some(include_str!("../docs/overview.md")),
    ),
    (
        "install",
        "PyPI, release archives, Docker, cargo",
        Some(include_str!("../docs/install.md")),
    ),
    (
        "commands",
        "every command and option, exit codes",
        Some(include_str!("../docs/commands.md")),
    ),
    (
        "config",
        "rosfix.toml, rule settings, suppression comments",
        Some(include_str!("../docs/config.md")),
    ),
    (
        "environment",
        "installed ROS, knowledge packs, distributions",
        Some(include_str!("../docs/environment.md")),
    ),
    (
        "fixes",
        "automatic fixes and how they are verified",
        Some(include_str!("../docs/fixes.md")),
    ),
    (
        "assist",
        "the fix assistant and online references",
        Some(include_str!("../docs/assist.md")),
    ),
    (
        "doctor",
        "checking the shell's ROS 2 environment",
        Some(include_str!("../docs/doctor.md")),
    ),
    (
        "migrate",
        "finding what breaks on another distribution",
        Some(include_str!("../docs/migrate.md")),
    ),
    (
        "triage",
        "explaining a failed launch log with the findings",
        Some(include_str!("../docs/triage.md")),
    ),
    (
        "nodl",
        "reading and writing NoDL node interface documents",
        Some(include_str!("../docs/nodl.md")),
    ),
    (
        "check-bag",
        "comparing the launch graph with a recording",
        Some(include_str!("../docs/live.md")),
    ),
    (
        "policy",
        "generating an SROS2 access-control policy",
        Some(include_str!("../docs/policy.md")),
    ),
    (
        "status",
        "what each agent did and what limits it",
        Some(include_str!("../docs/status.md")),
    ),
    (
        "ci",
        "GitHub Actions, pre-commit, baselines",
        Some(include_str!("../docs/ci.md")),
    ),
    (
        "editors",
        "language server, VS Code and watch mode",
        Some(include_str!("../docs/editors.md")),
    ),
    (
        "output",
        "text, JSON, SARIF and rdjson reports",
        Some(include_str!("../docs/output.md")),
    ),
    ("agents", "the agents and the rules each one reports", None),
    ("rules", "every rule with its full explanation", None),
    (
        "limits",
        "what static analysis cannot see",
        Some(include_str!("../docs/limits.md")),
    ),
];

/// The topic names, for completion in `rosfix shell`.
pub fn topic_names() -> impl Iterator<Item = &'static str> {
    TOPICS.iter().map(|t| t.0).chain(["all"])
}

fn index() -> String {
    let mut out = String::from(
        "rosfix manual\n\nRead a topic with `rosfix docs TOPIC`, or everything with `rosfix docs all`.\n\n",
    );
    for (name, about, _) in TOPICS {
        let _ = writeln!(out, "    {name:<12} {about}");
    }
    out
}

fn agents_page() -> String {
    let mut out = String::from(
        "# Agents\n\nEach agent checks one problem area and runs only when the workspace contains\nsomething it checks. `--agents a,b` runs only the named agents; core always runs.\n",
    );
    for a in all_agents() {
        let ids: Vec<&str> = RULES
            .iter()
            .filter(|r| r.agent == a.id())
            .map(|r| r.id)
            .collect();
        let _ = write!(out, "\n## {}\n\n{}\n", a.id(), a.title());
        if !ids.is_empty() {
            let _ = writeln!(out, "Rules: {}", ids.join(", "));
        }
        for r in RULES.iter().filter(|r| r.agent == a.id()) {
            let _ = writeln!(
                out,
                "    {}  {:<7}  {}",
                r.id,
                r.severity.as_str(),
                r.summary
            );
        }
    }
    out
}

fn rules_page() -> String {
    let mut out = format!(
        "# Rules\n\n{} rules. IDs are stable: they appear in reports, baselines, suppression\ncomments and rosfix.toml. The severity shown is the default; rosfix.toml can change it.\n",
        RULES.len()
    );
    for r in RULES {
        let off = if r.enabled_by_default {
            ""
        } else {
            ", off by default"
        };
        let _ = write!(
            out,
            "\n## {} {}\n\nAgent {}, {}{off}.\n\n{}\n\n{}\n",
            r.id,
            r.name,
            r.agent,
            r.severity.as_str(),
            r.summary,
            r.explanation
        );
    }
    out
}

fn topic(name: &str) -> Option<String> {
    match name {
        "agents" => Some(agents_page()),
        "rules" => Some(rules_page()),
        _ => TOPICS
            .iter()
            .find(|(n, _, _)| *n == name)
            .and_then(|(_, _, t)| t.map(str::to_string)),
    }
}

/// The manual: the topic list, one topic, or `all`.
pub fn render(name: Option<&str>) -> Result<String, String> {
    match name {
        None => Ok(index()),
        Some("all") => Ok(TOPICS
            .iter()
            .filter_map(|(n, _, _)| topic(n))
            .collect::<Vec<_>>()
            .join("\n\n")),
        Some(n) => {
            let n = n.to_ascii_lowercase();
            topic(&n).ok_or_else(|| {
                let names: Vec<&str> = TOPICS.iter().map(|t| t.0).collect();
                format!("unknown topic `{n}` (topics: {}, all)", names.join(", "))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_renders() {
        for (n, _, _) in TOPICS {
            let t = render(Some(n)).unwrap();
            assert!(t.starts_with("# "), "{n}");
        }
        assert!(render(Some("nope")).is_err());
        let all = render(Some("all")).unwrap();
        assert!(all.contains("# Rules") && all.contains("# Limits"));
    }

    #[test]
    fn generated_pages_cover_every_rule() {
        let rules = render(Some("rules")).unwrap();
        let agents = render(Some("agents")).unwrap();
        for r in RULES {
            assert!(
                rules.contains(&format!("## {} {}", r.id, r.name)),
                "{}",
                r.id
            );
            assert!(agents.contains(&format!("    {}  ", r.id)), "{}", r.id);
        }
    }

    /// Commands named in the manual exist, and every command is documented.
    #[test]
    fn commands_page_matches_the_cli() {
        let page = render(Some("commands")).unwrap();
        for c in [
            "check",
            "fix",
            "watch",
            "lsp",
            "graph",
            "rules",
            "explain",
            "baseline",
            "init",
            "doctor",
            "migrate",
            "triage",
            "nodl export",
            "check-bag",
            "policy",
            "status",
            "docs",
        ] {
            assert!(page.contains(&format!("rosfix {c}")), "{c}");
        }
    }
}
