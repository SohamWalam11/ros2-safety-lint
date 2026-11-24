//! `rosfix.toml` configuration.
//!
//! ```toml
//! ignore = ["third_party/**"]          # extra paths to skip (globs, relative to root)
//! test_paths = ["**/test/**"]          # executor rules skip these
//! external_topics = ["/scan"]          # published/subscribed outside the workspace
//! fail_on = "warning"                  # exit code 1 at/above this severity
//!
//! [rules]
//! P003 = "off"        # or "info" | "warning" | "error" | "on"
//! ```

use crate::rules::{self, Severity};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct RawConfig {
    pub ignore: Vec<String>,
    pub test_paths: Option<Vec<String>>,
    pub external_topics: Vec<String>,
    pub fail_on: Option<String>,
    pub rules: HashMap<String, String>,
    /// ROS distribution of this workspace (humble, jazzy, kilted, rolling).
    pub distro: Option<String>,
    /// Allow allowlisted web lookups when no local knowledge explains a finding.
    pub online: bool,
    /// Optional self-hosted SearXNG instance used by online lookups.
    pub searxng_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleSetting {
    Off,
    On(Severity),
}

#[derive(Debug, Clone)]
pub struct Config {
    pub ignore: GlobSet,
    pub test_paths: GlobSet,
    pub external_topics: Vec<String>,
    pub fail_on: Severity,
    pub rule_overrides: HashMap<&'static str, RuleSetting>,
    /// Where the config was loaded from, if anywhere.
    pub source: Option<String>,
    pub distro: Option<String>,
    pub online: bool,
    pub searxng_url: Option<String>,
}

/// Topics that are commonly provided by infrastructure outside any workspace.
pub const DEFAULT_EXTERNAL_TOPICS: &[&str] = &[
    "/tf",
    "/tf_static",
    "/clock",
    "/rosout",
    "/parameter_events",
    "/diagnostics",
    "/joy",
    "/cmd_vel",
    "/initialpose",
    "/goal_pose",
    "/clicked_point",
];

const DEFAULT_TEST_PATHS: &[&str] = &[
    "**/test/**",
    "**/tests/**",
    "**/test_*.cpp",
    "**/test_*.py",
    "**/*_test.cpp",
    "**/*_test.py",
];

impl Default for Config {
    fn default() -> Self {
        Config::from_raw(RawConfig::default(), None).expect("default config is valid")
    }
}

fn build_globs(patterns: &[String]) -> Result<GlobSet, String> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        let g = Glob::new(p).map_err(|e| format!("invalid glob `{p}`: {e}"))?;
        b.add(g);
    }
    b.build().map_err(|e| e.to_string())
}

impl Config {
    pub fn from_raw(raw: RawConfig, source: Option<String>) -> Result<Self, String> {
        let tests: Vec<String> = raw
            .test_paths
            .clone()
            .unwrap_or_else(|| DEFAULT_TEST_PATHS.iter().map(|s| s.to_string()).collect());
        let mut overrides = HashMap::new();
        for (k, v) in &raw.rules {
            let rule = rules::find(k).ok_or_else(|| format!("unknown rule `{k}` in [rules]"))?;
            let setting = match v.to_ascii_lowercase().as_str() {
                "off" | "false" | "disable" | "disabled" => RuleSetting::Off,
                "on" | "true" | "enable" | "enabled" => RuleSetting::On(rule.severity),
                other => RuleSetting::On(
                    Severity::parse(other)
                        .ok_or_else(|| format!("invalid setting `{v}` for rule {k}"))?,
                ),
            };
            overrides.insert(rule.id, setting);
        }
        let fail_on = match &raw.fail_on {
            Some(s) => Severity::parse(s).ok_or_else(|| format!("invalid fail_on `{s}`"))?,
            None => Severity::Warning,
        };
        let mut external: Vec<String> = DEFAULT_EXTERNAL_TOPICS
            .iter()
            .map(|s| s.to_string())
            .collect();
        external.extend(raw.external_topics.iter().cloned());
        Ok(Config {
            ignore: build_globs(&raw.ignore)?,
            test_paths: build_globs(&tests)?,
            external_topics: external,
            fail_on,
            rule_overrides: overrides,
            source,
            distro: raw.distro.clone(),
            online: raw.online,
            searxng_url: raw.searxng_url.clone(),
        })
    }

    /// Loads `rosfix.toml` from `dir` (or the legacy `ros2-safety.toml`).
    /// A malformed config is an error, never silently replaced by defaults.
    pub fn load_from_dir(dir: &Path) -> Result<Self, String> {
        for name in ["rosfix.toml", "ros2-safety.toml"] {
            let p = dir.join(name);
            if p.is_file() {
                let text =
                    std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                let raw: RawConfig =
                    toml::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
                return Config::from_raw(raw, Some(p.display().to_string()));
            }
        }
        Ok(Config::default())
    }

    /// Effective setting for a rule, after config overrides.
    pub fn rule_setting(&self, id: &str) -> RuleSetting {
        let rule = rules::find(id).expect("known rule");
        if let Some(s) = self.rule_overrides.get(rule.id) {
            return *s;
        }
        if rule.enabled_by_default {
            RuleSetting::On(rule.severity)
        } else {
            RuleSetting::Off
        }
    }

    pub fn is_test_path(&self, rel: &str) -> bool {
        self.test_paths.is_match(rel)
    }

    pub fn is_external_topic(&self, name: &str) -> bool {
        self.external_topics.iter().any(|t| t == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rules_and_rejects_unknown() {
        let raw: RawConfig = toml::from_str("[rules]\nP003 = \"off\"\nQ001 = \"warning\"").unwrap();
        let c = Config::from_raw(raw, None).unwrap();
        assert_eq!(c.rule_setting("P003"), RuleSetting::Off);
        assert_eq!(c.rule_setting("Q001"), RuleSetting::On(Severity::Warning));
        let bad: RawConfig = toml::from_str("[rules]\nZZZ = \"off\"").unwrap();
        assert!(Config::from_raw(bad, None).is_err());
    }

    #[test]
    fn unknown_top_level_key_is_error() {
        assert!(toml::from_str::<RawConfig>("ignore_pathz = []").is_err());
    }

    #[test]
    fn test_path_globs() {
        let c = Config::default();
        assert!(c.is_test_path("pkg/test/test_x.cpp"));
        assert!(!c.is_test_path("pkg/src/x.cpp"));
    }
}
