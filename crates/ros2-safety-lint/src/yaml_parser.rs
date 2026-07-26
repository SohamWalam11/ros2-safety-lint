#![allow(clippy::only_used_in_recursion)]
use crate::sros2::LintViolation;
use serde::Deserialize;
use serde_yaml::Value;

pub fn lint_yaml(content: &str) -> Vec<LintViolation> {
    let mut violations = Vec::new();

    // Parse YAML, it can be a stream of documents
    for document in serde_yaml::Deserializer::from_str(content) {
        if let Ok(value) = Value::deserialize(document) {
            walk_yaml(&value, &mut violations, content);
        }
    }

    violations
}

fn walk_yaml(root_value: &Value, violations: &mut Vec<LintViolation>, _content: &str) {
    let mut stack = vec![root_value];

    while let Some(value) = stack.pop() {
        match value {
            Value::Mapping(map) => {
                for (k, v) in map {
                    if let Value::String(key_str) = k {
                        let key_lower = key_str.to_lowercase();

                        if key_lower == "domain_id" {
                            if let Value::Number(num) = v {
                                if num.as_i64() == Some(0) {
                                    violations.push(LintViolation {
                                        message: "Unsafe ROS_DOMAIN_ID 0 detected. This causes cross-talk on shared networks.".to_string(),
                                        range: 0..1,
                                    });
                                }
                            }
                        } else if key_lower == "reliability" {
                            if let Value::String(val_str) = v {
                                if val_str.to_lowercase() == "best_effort" {
                                    violations.push(LintViolation {
                                        message: "QoS Reliability set to 'best_effort'. Ensure this is only used for high-frequency sensor data, not critical state.".to_string(),
                                        range: 0..1,
                                    });
                                }
                            }
                        }
                    }
                    stack.push(v);
                }
            }
            Value::Sequence(seq) => {
                for v in seq {
                    stack.push(v);
                }
            }
            _ => {}
        }
    }
}
