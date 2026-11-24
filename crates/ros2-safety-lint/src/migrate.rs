//! `rosfix migrate`: what a workspace gets wrong on another ROS distribution.
//!
//! The workspace is analysed twice, with the knowledge pack of the current distribution and
//! with the pack of the target, and only the findings that are new on the target are kept:
//! parameters that upstream nodes removed, renamed or retyped, executables and plugins that
//! no longer exist. Findings the workspace already has on its current distribution are not
//! migration problems and are left out.
//!
//! The installed ROS environment is ignored in both runs: it belongs to one distribution,
//! so it would make both runs see the same packages.

use crate::config::Config;
use crate::engine::{self, Analysis, Options};
use std::collections::HashSet;
use std::path::Path;

/// Distributions with a knowledge pack (`rolling` uses the newest pack).
pub const DISTROS: &[&str] = &["humble", "jazzy", "kilted", "rolling"];

pub struct Migration {
    pub from: String,
    pub to: String,
    /// The analysis on the target distribution, reduced to the new findings.
    pub after: Analysis,
    /// Options of the target run (for fix verification).
    pub opts: Options,
    /// Findings on the current distribution that the target no longer reports.
    pub resolved: usize,
    /// Findings reported on both distributions (not shown).
    pub unchanged: usize,
}

fn pack_of(d: &str) -> &str {
    if d == "rolling" {
        "kilted"
    } else {
        d
    }
}

pub fn run(
    root: &Path,
    config: &Config,
    base: &Options,
    from: &str,
    to: &str,
) -> Result<Migration, String> {
    for d in [from, to] {
        if !DISTROS.contains(&d) {
            return Err(format!(
                "unknown distribution `{d}` (known: {})",
                DISTROS.join(", ")
            ));
        }
    }
    if pack_of(from) == pack_of(to) {
        return Err(format!(
            "`{from}` and `{to}` use the same knowledge pack; there is nothing to compare"
        ));
    }
    let with = |d: &str| {
        let mut o = base.clone();
        o.distro = Some(d.to_string());
        o.prefixes = Vec::new();
        o.previous = None;
        o
    };
    let before = engine::analyze(root, config, &with(from));
    let opts = with(to);
    let mut after = engine::analyze(root, config, &opts);
    let old: HashSet<String> = before
        .diagnostics
        .iter()
        .map(|d| engine::fingerprint(&before.index, d))
        .collect();
    let now: HashSet<String> = after
        .diagnostics
        .iter()
        .map(|d| engine::fingerprint(&after.index, d))
        .collect();
    let total = after.diagnostics.len();
    let ix = &after.index;
    after
        .diagnostics
        .retain(|d| !old.contains(&engine::fingerprint(ix, d)));
    Ok(Migration {
        from: from.to_string(),
        to: to.to_string(),
        unchanged: total - after.diagnostics.len(),
        resolved: old.difference(&now).count(),
        after,
        opts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rosfix-migrate-{}-{}",
            std::process::id(),
            files.len()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        for (p, c) in files {
            let path = dir.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, c).unwrap();
        }
        dir
    }

    #[test]
    fn rejects_unknown_and_identical_distributions() {
        let dir = workspace(&[]);
        let o = Options::default();
        let c = Config::default();
        assert!(run(&dir, &c, &o, "humble", "foxy").is_err());
        assert!(run(&dir, &c, &o, "kilted", "rolling").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `nav2_behaviors/behavior_server` renamed `costmap_topic` to `local_costmap_topic`
    /// between Humble and Jazzy (packs; see research/data/s3_drift.md).
    #[test]
    fn reports_a_parameter_removed_upstream() {
        let launch = "from launch import LaunchDescription\nfrom launch_ros.actions import Node\nimport os\nfrom ament_index_python.packages import get_package_share_directory\ndef generate_launch_description():\n    cfg = os.path.join(get_package_share_directory('bringup'), 'config', 'behavior.yaml')\n    return LaunchDescription([\n        Node(package='nav2_behaviors', executable='behavior_server', name='behavior_server', parameters=[cfg]),\n    ])\n";
        let dir = workspace(&[
            ("bringup/package.xml", "<package format=\"3\"><name>bringup</name><exec_depend>nav2_behaviors</exec_depend></package>"),
            ("bringup/launch/nav.launch.py", launch),
            ("bringup/config/behavior.yaml", "behavior_server:\n  ros__parameters:\n    costmap_topic: local_costmap/costmap_raw\n"),
        ]);
        let m = run(
            &dir,
            &Config::default(),
            &Options::default(),
            "humble",
            "jazzy",
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let new: Vec<_> = m
            .after
            .diagnostics
            .iter()
            .map(|d| (d.rule, d.message.clone()))
            .collect();
        assert!(
            new.iter()
                .any(|(r, msg)| matches!(*r, "P003" | "P007") && msg.contains("costmap_topic")),
            "{new:#?}"
        );
        // Moving back reports nothing for this file: the parameter exists on Humble.
        assert!(m.from == "humble" && m.to == "jazzy");
    }
}
