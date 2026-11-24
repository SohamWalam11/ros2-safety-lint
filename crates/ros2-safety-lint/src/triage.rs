//! Log triage: explain runtime errors from a `ros2 launch` / node log with the static findings
//! that cause them ("find before running" → "explain after it failed").
//!
//! Each recognised log line yields an [`Explained`]: what the error means, the findings of the
//! analysis that explain it (rule, location, message), and a hint when no finding matches.

use crate::engine::Analysis;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub rule: &'static str,
    /// `file:line:col`
    pub location: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Explained {
    /// 1-based line in the log.
    pub line: usize,
    pub log: String,
    /// What kind of failure this is.
    pub problem: &'static str,
    /// The name the error is about (node, class, topic, parameter, …).
    pub subject: String,
    /// Static findings that explain it (empty if none matched).
    pub findings: Vec<Finding>,
    /// What to check when no finding matched.
    pub hint: &'static str,
}

struct Pattern {
    re: &'static str,
    problem: &'static str,
    /// Rules whose findings can explain it; a finding must also mention the subject.
    rules: &'static [&'static str],
    hint: &'static str,
}

const PATTERNS: &[Pattern] = &[
    Pattern {
        re: r"class (\S+?) with base class type \S+ does not exist",
        problem: "a plugin class name that pluginlib cannot find",
        rules: &["N002", "N008", "C003", "L001", "L006", "L010"],
        hint: "Check the `plugin:` value against the plugins of your ROS distribution, and that the plugin's package is installed and exports it.",
    },
    Pattern {
        re: r"Failed to find class with the requested plugin name '([^']+)'",
        problem: "a component that the container cannot load",
        rules: &["L001", "L006", "L010"],
        hint: "Check `rclcpp_components_register_nodes`, `RCLCPP_COMPONENTS_REGISTER_NODE` and `install(TARGETS …)` of the component's package.",
    },
    Pattern {
        re: r"Failed to change state for node:? ?'?([\w/]+)",
        problem: "a lifecycle node that failed to configure or activate",
        rules: &["M001", "M002", "N001", "N002", "N003", "N004", "N006", "N008", "N009", "N010", "P003", "P004", "P006", "P007"],
        hint: "The node's own log lines above this one say why; check its parameters and plugins.",
    },
    Pattern {
        re: r"launch configuration '([^']+)' does not exist",
        problem: "a launch argument that is used but never declared",
        rules: &["A001"],
        hint: "Declare it with DeclareLaunchArgument / <arg>, or pass it on the command line.",
    },
    Pattern {
        re: r"executable '([^']+)' not found",
        problem: "an executable the package does not build or install",
        rules: &["L001", "L003"],
        hint: "Check add_executable/install(TARGETS … DESTINATION lib/${PROJECT_NAME}) or console_scripts in setup.py/setup.cfg.",
    },
    Pattern {
        re: r"[Pp]ackage '([^']+)' not found",
        problem: "a package that is not installed or not declared",
        rules: &["L001", "L002", "L009"],
        hint: "Install it (rosdep install --from-paths src) and declare it in package.xml.",
    },
    Pattern {
        re: r"Cannot locate rosdep definition for \[([^\]]+)\]",
        problem: "a package.xml dependency rosdep cannot resolve",
        rules: &["L009"],
        hint: "Check the name in package.xml against rosdistro / rosdep keys.",
    },
    Pattern {
        re: r"(?:requesting|offering) incompatible QoS.*?topic '([^']+)'",
        problem: "publisher and subscriber QoS that are incompatible (no messages delivered)",
        rules: &["Q001", "Q002", "Q003", "Q005"],
        hint: "Align reliability/durability of both endpoints, or use qos_overrides where the node allows them.",
    },
    Pattern {
        re: r"incompatible QoS.*?topic\s+'?([/\w]+)",
        problem: "publisher and subscriber QoS that are incompatible (no messages delivered)",
        rules: &["Q001", "Q002", "Q003", "Q005"],
        hint: "Align reliability/durability of both endpoints.",
    },
    Pattern {
        re: r"Node not recognized:?\s*(\w+)",
        problem: "a behavior-tree node type that no loaded BT plugin provides",
        rules: &["N009", "N010"],
        hint: "Check the node type's spelling and that its plugin library is in bt_navigator's plugin_lib_names.",
    },
    Pattern {
        re: r"(?:Could not load controller with name|controller with name) '([^']+)'",
        problem: "a controller the controller manager does not know",
        rules: &["C003", "C007", "C009"],
        hint: "Define `<name>.type` in the controller manager's parameters or pass --param-file to the spawner.",
    },
    Pattern {
        re: r#"(?:Invalid frame ID|frame) "?'?([\w/]+)"?'? (?:passed to|does not exist)"#,
        problem: "a TF frame that does not exist",
        rules: &["F001", "F003", "F004", "N003"],
        hint: "Check the frame name against the robot description and static transforms, and that something publishes it.",
    },
    Pattern {
        re: r"parameter '([^']+)' has invalid type",
        problem: "a parameter value of the wrong type",
        rules: &["P004"],
        hint: "Write the value with the declared type (e.g. 1.0 for a double).",
    },
    Pattern {
        re: r"[Pp]arameter '?([\w.]+)'? (?:was not declared|is not declared|has not been declared)",
        problem: "a parameter the node does not declare",
        rules: &["P003", "P007"],
        hint: "Check the name against the node's declared parameters (it may have been renamed).",
    },
];

fn compiled() -> &'static [(regex::Regex, &'static Pattern)] {
    static RE: std::sync::OnceLock<Vec<(regex::Regex, &'static Pattern)>> =
        std::sync::OnceLock::new();
    RE.get_or_init(|| {
        PATTERNS
            .iter()
            .map(|p| (regex::Regex::new(p.re).expect("valid pattern"), p))
            .collect()
    })
}

/// The findings of `a` that explain an error about `subject`: findings of the pattern's rules
/// whose message mentions the subject (or its last path segment, for node and topic names).
fn related(a: &Analysis, rules: &[&str], subject: &str) -> Vec<Finding> {
    let leaf = subject.rsplit('/').next().unwrap_or(subject);
    let mentions = |m: &str| {
        m.contains(&format!("`{subject}`"))
            || m.contains(subject)
            || (!leaf.is_empty() && m.contains(&format!("`{leaf}`")))
    };
    a.diagnostics
        .iter()
        .filter(|d| rules.contains(&d.rule) && mentions(&d.message))
        .map(|d| {
            let f = a.index.file(d.loc.file);
            let (l, c) = f.line_col(d.loc.start);
            Finding {
                rule: d.rule,
                location: format!("{}:{l}:{c}", f.rel_path),
                message: d.message.clone(),
            }
        })
        .collect()
}

/// Explains every recognised error line of `log` with the findings of `analysis`.
pub fn run(log: &str, analysis: &Analysis) -> Vec<Explained> {
    let mut out = Vec::new();
    for (i, line) in log.lines().enumerate() {
        for (re, p) in compiled() {
            let Some(c) = re.captures(line) else { continue };
            let subject = c
                .get(1)
                .map(|m| {
                    m.as_str()
                        .trim_matches(|ch| ch == '\'' || ch == '"')
                        .to_string()
                })
                .unwrap_or_default();
            out.push(Explained {
                line: i + 1,
                log: line.trim().chars().take(300).collect(),
                problem: p.problem,
                findings: related(analysis, p.rules, &subject),
                subject,
                hint: p.hint,
            });
            break; // one explanation per log line
        }
    }
    out
}

/// Plain-text rendering (the CLI's default).
pub fn render(items: &[Explained]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for e in items {
        let _ = writeln!(
            s,
            "log line {}: {}\n  → {} (`{}`)",
            e.line, e.log, e.problem, e.subject
        );
        if e.findings.is_empty() {
            let _ = writeln!(s, "  no static finding explains it; {}", e.hint);
        }
        for f in &e.findings {
            let _ = writeln!(s, "  {} {}: {}", f.rule, f.location, f.message);
        }
        s.push('\n');
    }
    let explained = items.iter().filter(|e| !e.findings.is_empty()).count();
    let _ = writeln!(
        s,
        "{} error line(s) recognised, {explained} explained by a finding",
        items.len()
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::engine::{analyze, Options};

    #[test]
    fn explains_runtime_errors_with_findings() {
        let dir = tempfile::tempdir().unwrap();
        let w = |r: &str, t: &str| {
            let p = dir.path().join(r);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        w(
            "p/package.xml",
            "<package format=\"3\"><name>p</name></package>",
        );
        w(
            "p/config/nav2.yaml",
            "amcl:\n  ros__parameters:\n    robot_model_type: \"differential\"\n",
        );
        w("p/launch/a.launch.py", "from launch import LaunchDescription\nfrom launch.substitutions import LaunchConfiguration\nfrom launch_ros.actions import Node\ndef generate_launch_description():\n    return LaunchDescription([Node(package='p', executable='x', parameters=[{'use_sim_time': LaunchConfiguration('use_sim_tme')}])])\n");
        let a = analyze(
            dir.path(),
            &Config::default(),
            &Options {
                distro: Some("jazzy".into()),
                ..Default::default()
            },
        );
        let log = "[INFO] [launch]: All log files can be found below /root/.ros/log\n\
[ERROR] [launch]: Caught exception in launch (see debug for traceback): launch configuration 'use_sim_tme' does not exist\n\
[amcl-1] [FATAL] [amcl]: Failed to create motion model: class differential with base class type nav2_amcl::MotionModel does not exist. Declared types are nav2_amcl::DifferentialMotionModel\n\
[lifecycle_manager-2] [ERROR] [lifecycle_manager_localization]: Failed to change state for node: amcl\n\
[ERROR] [something]: unrelated message\n";
        let r = run(log, &a);
        assert_eq!(r.len(), 3, "{r:#?}");
        assert!(
            r[0].problem.contains("launch argument")
                && r[0].findings.iter().any(|f| f.rule == "A001"),
            "{r:#?}"
        );
        assert!(
            r[1].subject == "differential" && r[1].findings.iter().any(|f| f.rule == "N008"),
            "{r:#?}"
        );
        assert_eq!(r[2].subject, "amcl");
        let text = render(&r);
        assert!(text.contains("3 error line(s) recognised"), "{text}");
    }
}
