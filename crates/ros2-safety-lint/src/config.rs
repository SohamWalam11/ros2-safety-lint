use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
pub struct SafetyConfig {
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
