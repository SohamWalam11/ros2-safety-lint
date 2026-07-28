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
                        } else if key_lower == "robot_radius" {
                            if let Value::Number(num) = v {
                                if let Some(r) = num.as_f64() {
                                    if r <= 0.0 {
                                        violations.push(LintViolation {
                                            message: "Nav2 Physical Safety Hazard: 'robot_radius' set to <= 0.0. Robot will collide with obstacles.".to_string(),
                                            range: 0..1,
                                        });
                                    }
                                }
                            }
                        } else if key_lower == "inflation_radius" {
                            if let Value::Number(num) = v {
                                if let Some(r) = num.as_f64() {
                                    if r < 0.05 {
                                        violations.push(LintViolation {
                                            message: "Nav2 Physical Safety Hazard: 'inflation_radius' set dangerously low (< 0.05m).".to_string(),
                                            range: 0..1,
                                        });
                                    }
                                }
                            }
                        } else if key_lower == "footprint" {
                            if let Value::Sequence(seq) = v {
                                if seq.is_empty() {
                                    violations.push(LintViolation {
                                        message: "Nav2 Physical Safety Hazard: 'footprint' is empty []. Costmap will fail to compute physical collision bounds.".to_string(),
                                        range: 0..1,
                                    });
                                }
                            } else if let Value::String(s) = v {
                                if s.trim() == "[]" || s.trim().is_empty() {
                                    violations.push(LintViolation {
                                        message: "Nav2 Physical Safety Hazard: 'footprint' is empty []. Costmap will fail to compute physical collision bounds.".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yaml_domain_id_zero() {
        let yaml = "ros__parameters:\n  domain_id: 0\n";
        let violations = lint_yaml(yaml);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("ROS_DOMAIN_ID 0"));
    }

    #[test]
    fn test_yaml_reliability_best_effort() {
        let yaml = "ros__parameters:\n  reliability: best_effort\n";
        let violations = lint_yaml(yaml);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("best_effort"));
    }

    #[test]
    fn test_yaml_nav2_robot_radius_zero() {
        let yaml = "local_costmap:\n  ros__parameters:\n    robot_radius: 0.0\n";
        let violations = lint_yaml(yaml);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("robot_radius"));
    }

    #[test]
    fn test_yaml_clean() {
        let yaml = "ros__parameters:\n  domain_id: 42\n  reliability: reliable\n";
        let violations = lint_yaml(yaml);
        assert_eq!(violations.len(), 0);
    }
}

