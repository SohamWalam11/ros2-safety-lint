use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
pub struct SafetyConfig {
    #[serde(default)]
    pub ignore_paths: Vec<String>,
    pub rules: Rules,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Rules {
    pub require_encryption: Option<bool>,
    pub max_qos_history: Option<usize>,
    pub allow_best_effort: Option<bool>,
}

impl Default for SafetyConfig {
    fn default() -> Self {
        SafetyConfig {
            ignore_paths: vec!["target".to_string(), ".git".to_string(), "vendor".to_string()],
            rules: Rules {
                require_encryption: Some(true),
                max_qos_history: Some(10),
                allow_best_effort: Some(false),
            },
        }
    }
}

pub fn load_config<P: AsRef<Path>>(path: P) -> SafetyConfig {
    if let Ok(content) = fs::read_to_string(path) {
        if let Ok(config) = toml::from_str(&content) {
            return config;
        }
    }
    SafetyConfig::default()
}

pub fn find_and_load_config<P: AsRef<Path>>(workspace_dir: P) -> SafetyConfig {
    let rosfix_toml = workspace_dir.as_ref().join("rosfix.toml");
    if rosfix_toml.exists() {
        return load_config(rosfix_toml);
    }
    let legacy_toml = workspace_dir.as_ref().join("ros2-safety.toml");
    if legacy_toml.exists() {
        load_config(legacy_toml)
    } else {
        SafetyConfig::default()
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let cfg = SafetyConfig::default();
        assert_eq!(cfg.rules.require_encryption, Some(true));
        assert!(cfg.ignore_paths.contains(&"target".to_string()));
    }

    #[test]
    fn test_config_from_toml() {
        let toml_str = r#"
            ignore_paths = ["build", "install"]
            [rules]
            require_encryption = false
            max_qos_history = 50
            allow_best_effort = true
        "#;
        let cfg: SafetyConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.ignore_paths, vec!["build", "install"]);
        assert_eq!(cfg.rules.require_encryption, Some(false));
        assert_eq!(cfg.rules.max_qos_history, Some(50));
    }
}


