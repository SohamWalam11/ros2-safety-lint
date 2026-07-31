use crate::sros2::LintViolation;
use crate::blackboard::{AgentTask, AgentDomain, BlackboardEventBus};
use std::collections::HashMap;


/// Severity level of a detected lint violation after semantic context analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViolationSeverity {
    /// Critical architectural bypass in a safety-critical pipeline (e.g. perception, actuation, control)
    Critical,
    /// Benign or telemetry exception (e.g. diagnostic logging, UI counters, low-priority monitoring)
    Benign,
}

/// Semantic context extracted from file path, AST scope, and node/topic declarations.
#[derive(Debug, Clone)]
pub struct SemanticContext {
    pub file_path: String,
    pub node_name: Option<String>,
    pub topic_name: Option<String>,
    pub message_type: Option<String>,
}

impl SemanticContext {
    pub fn from_file_path_and_content(file_path: &str, content: &str) -> Self {
        let file_path_lower = file_path.to_lowercase();

        // Extract topic name heuristic if available in text snippet
        let topic_name = if content.contains("/perception/") {
            Some("/perception/obstacle_bbox".to_string())
        } else if content.contains("/control/") || content.contains("cmd_vel") {
            Some("/control/cmd_vel".to_string())
        } else if content.contains("/diagnostics") || content.contains("diagnostic") {
            Some("/diagnostics/gpu_temp".to_string())
        } else {
            None
        };

        // Extract message type heuristic
        let message_type = if file_path_lower.contains("diagnostic") || content.contains("DiagnosticArray") || content.contains("DiagnosticStatus") {
            Some("diagnostic_msgs/msg/DiagnosticArray".to_string())
        } else if content.contains("LaserScan") || content.contains("PointCloud2") {
            Some("sensor_msgs/msg/LaserScan".to_string())
        } else if content.contains("Twist") || content.contains("Odometry") {
            Some("geometry_msgs/msg/Twist".to_string())
        } else {
            None
        };

        // Node name heuristic
        let node_name = if file_path_lower.contains("nav2") || file_path_lower.contains("autoware") || file_path_lower.contains("planner") || file_path_lower.contains("controller") {
            Some("safety_critical_node".to_string())
        } else if file_path_lower.contains("diag") || file_path_lower.contains("telemetry") || file_path_lower.contains("monitor") {
            Some("telemetry_logger_node".to_string())
        } else {
            None
        };

        SemanticContext {
            file_path: file_path.to_string(),
            node_name,
            topic_name,
            message_type,
        }
    }
}

/// Evaluates a lint violation within its semantic context to determine whether it is critical
/// or a benign telemetry exception.
pub fn classify_violation(violation: &LintViolation, context: &SemanticContext) -> ViolationSeverity {
    let file_path_lower = context.file_path.to_lowercase();
    let msg_lower = violation.message.to_lowercase();

    // 1. Benign Diagnostic/Telemetry Filtering
    if file_path_lower.contains("diagnostic") 
        || file_path_lower.contains("telemetry") 
        || file_path_lower.contains("logging") 
        || file_path_lower.contains("monitor")
    {
        return ViolationSeverity::Benign;
    }

    if let Some(ref msg_type) = context.message_type {
        if msg_type.contains("diagnostic_msgs") || msg_type.contains("rosgraph_msgs") {
            return ViolationSeverity::Benign;
        }
    }

    if let Some(ref topic) = context.topic_name {
        if topic.contains("/diagnostics") || topic.contains("/telemetry") || topic.contains("/debug") {
            return ViolationSeverity::Benign;
        }
    }

    // 2. Safety-Critical Pathway Overrides (Perception, Planning, Control, Security)
    if msg_lower.contains("best_effort") || msg_lower.contains("wildcard") || msg_lower.contains("rtps_protection_kind") {
        if file_path_lower.contains("nav2") 
            || file_path_lower.contains("autoware") 
            || file_path_lower.contains("moveit")
            || file_path_lower.contains("perception")
            || file_path_lower.contains("control")
            || file_path_lower.contains("governance")
            || file_path_lower.contains("permissions")
        {
            return ViolationSeverity::Critical;
        }
    }

    // Default to Critical for safety verification safety margin
    ViolationSeverity::Critical
}

/// Filters out benign violations when semantic filtering is enabled.
pub fn filter_violations(
    violations: Vec<(String, LintViolation, String)>,
) -> (Vec<(String, LintViolation, String)>, usize) {
    let mut critical_violations = Vec::new();
    let mut benign_count = 0;

    for (file_path, violation, content) in violations {
        let context = SemanticContext::from_file_path_and_content(&file_path, &content);
        match classify_violation(&violation, &context) {
            ViolationSeverity::Critical => {
                critical_violations.push((file_path, violation, content));
            }
            ViolationSeverity::Benign => {
                benign_count += 1;
            }
        }
    }

    (critical_violations, benign_count)
}

/// Takes critical violations and posts them to the MAS Blackboard for Agents to claim.
pub fn broadcast_to_blackboard(
    critical_violations: &[(String, LintViolation, String)],
    blackboard: &mut BlackboardEventBus,
) {
    for (i, (file_path, violation, content)) in critical_violations.iter().enumerate() {
        let context = SemanticContext::from_file_path_and_content(file_path, content);
        
        // Very basic routing logic just for groundwork demonstration
        let domain = if violation.message.contains("BEST_EFFORT") || violation.message.contains("QoS") {
            AgentDomain::QoS
        } else if violation.message.contains("malloc") || violation.message.contains("new") || violation.message.contains("spin") || violation.message.contains("get()") || violation.message.contains("wait_for") || violation.message.contains("sleep") {
            AgentDomain::Executor
        } else if violation.message.contains("wildcard") || violation.message.contains("permission") || violation.message.contains("rtps_protection_kind") {
            AgentDomain::Security
        } else if violation.message.contains("package.xml") || violation.message.contains("CMake") {
            AgentDomain::BuildSystem
        } else if violation.message.contains("URDF") || violation.message.contains("footprint") {
            AgentDomain::Kinematics
        } else if violation.message.contains("Lifecycle") {
            AgentDomain::Lifecycle
        } else {
            AgentDomain::Semantic
        };

        let mut metadata = HashMap::new();
        if let Some(node) = context.node_name { metadata.insert("node_name".to_string(), node); }
        if let Some(topic) = context.topic_name { metadata.insert("topic_name".to_string(), topic); }

        let task = AgentTask {
            task_id: format!("TASK-{}", i), // Simplified for determinism in this stub
            target_domain: domain,
            file_path: file_path.clone(),
            violation_context: content.clone(),
            diagnostic_message: violation.message.clone(),
            start_byte: violation.range.start,
            end_byte: violation.range.end,
            semantic_metadata: metadata,
        };

        blackboard.post_task(task);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sros2::LintViolation;

    #[test]
    fn test_diagnostic_benign_classification() {
        let violation = LintViolation {
            message: "Hardcoded BEST_EFFORT detected".to_string(),
            range: 0..10,
        };
        let context = SemanticContext::from_file_path_and_content("diagnostic_logger.cpp", "DiagnosticArray");
        assert_eq!(classify_violation(&violation, &context), ViolationSeverity::Benign);
    }

    #[test]
    fn test_nav2_critical_classification() {
        let violation = LintViolation {
            message: "Hardcoded BEST_EFFORT detected".to_string(),
            range: 0..10,
        };
        let context = SemanticContext::from_file_path_and_content("autoware_perception_node.cpp", "LaserScan");
        assert_eq!(classify_violation(&violation, &context), ViolationSeverity::Critical);
    }
}

