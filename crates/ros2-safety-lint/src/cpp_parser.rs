use crate::sros2::LintViolation;
use tree_sitter::{Node, Parser};

/// Checks C++ source files for hardcoded QoS profiles and safety violations.
use std::collections::HashSet;

pub fn lint_cpp(content: &str) -> Vec<LintViolation> {
    let mut violations = Vec::new();
    let mut tainted_vars = HashSet::new();

    let mut parser = Parser::new();
    // Safety: The language function is FFI, but trusted.
    parser
        .set_language(tree_sitter_cpp::language())
        .expect("Error loading C++ grammar");

    if let Some(tree) = parser.parse(content, None) {
        let root_node = tree.root_node();
        walk_node(
            &root_node,
            content.as_bytes(),
            &mut violations,
            &mut tainted_vars,
        );
    }

    violations
}

fn walk_node(
    root_node: &Node,
    source: &[u8],
    violations: &mut Vec<LintViolation>,
    tainted_vars: &mut HashSet<String>,
) {
    let mut stack = vec![(*root_node, false)];

    while let Some((node, inside_callback)) = stack.pop() {
        let is_lambda_or_cb = inside_callback
            || node.kind() == "lambda_expression"
            || node.kind() == "arrow_field_expression";

        // Taint Tracking: If we see a variable initialized with QoS, taint it
        if node.kind() == "declaration" {
            if let Ok(text) = node.utf8_text(source) {
                if text.contains("rclcpp::QoS") {
                    tainted_vars.insert(text.to_string());
                }
            }
        }

        // Check if this node is an identifier or field that might represent best_effort
        if node.kind() == "identifier" || node.kind() == "field_identifier" {
            if let Ok(text) = node.utf8_text(source) {
                if text == "best_effort" || text == "BEST_EFFORT" {
                    violations.push(LintViolation {
                        message: "Hardcoded BEST_EFFORT QoS profile detected via Taint Tracking. This circumvents architectural safety and can cause silent data loss.".to_string(),
                        range: node.start_byte()..node.end_byte(),
                    });
                }
            }
        }

        // Real-Time Control Loop Heap Allocation Taint Tracking
        if node.kind() == "call_expression" || node.kind() == "new_expression" {
            if let Ok(text) = node.utf8_text(source) {
                if is_lambda_or_cb && (text.contains("make_shared") || text.contains("make_unique") || text.contains("malloc") || text.contains("new ") || text.contains("push_back")) {
                    violations.push(LintViolation {
                        message: "Real-Time Hazard: Dynamic heap memory allocation (make_shared/malloc/new/push_back) detected inside high-frequency callback/control loop. This breaks non-deterministic real-time guarantees.".to_string(),
                        range: node.start_byte()..node.end_byte(),
                    });
                }
            }
        }

        // Executor Deadlock Detection in C++
        if node.kind() == "call_expression" {
            if let Ok(text) = node.utf8_text(source) {
                if text.contains("spin_until_future_complete") {
                    violations.push(LintViolation {
                        message: "Executor Deadlock Risk: 'spin_until_future_complete' called inside ROS 2 node. Avoid nested spinning on single-threaded executors.".to_string(),
                        range: node.start_byte()..node.end_byte(),
                    });
                } else if is_lambda_or_cb && (text.contains(".get()") || text.contains("sleep_for") || text.contains(".wait_for(")) {
                    violations.push(LintViolation {
                        message: "Executor Deadlock Risk: Blocking wait/sleep call detected inside callback scope. This will freeze single-threaded ROS 2 executors.".to_string(),
                        range: node.start_byte()..node.end_byte(),
                    });
                }
            }
        }

        // Push children to stack
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push((child, is_lambda_or_cb));
        }

    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpp_best_effort_detection() {
        let code = "auto qos = rclcpp::QoS(10).best_effort();\n";
        let violations = lint_cpp(code);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("BEST_EFFORT"));
    }

    #[test]
    fn test_cpp_executor_deadlock_spin() {
        let code = "rclcpp::spin_until_future_complete(node, future);\n";
        let violations = lint_cpp(code);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("Executor Deadlock Risk"));
    }

    #[test]
    fn test_cpp_executor_deadlock_lambda_get() {
        let code = "auto sub = node->create_subscription<std_msgs::msg::String>(\"topic\", 10, [](std_msgs::msg::String::SharedPtr msg) { auto res = future.get(); });\n";
        let violations = lint_cpp(code);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("Executor Deadlock Risk"));
    }

    #[test]
    fn test_cpp_clean_code() {
        let code = "auto qos = rclcpp::QoS(10).reliable();\n";
        let violations = lint_cpp(code);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_cpp_executor_deadlock_wait_for() {
        let code = "auto sub = node->create_subscription<std_msgs::msg::String>(\"topic\", 10, [](auto msg) { future.wait_for(std::chrono::seconds(1)); });\n";
        let violations = lint_cpp(code);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("Executor Deadlock Risk"));
    }

    #[test]
    fn test_cpp_executor_deadlock_sleep_for() {
        let code = "auto sub = node->create_subscription<std_msgs::msg::String>(\"topic\", 10, [](auto msg) { std::this_thread::sleep_for(std::chrono::seconds(2)); });\n";
        let violations = lint_cpp(code);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("Executor Deadlock Risk"));
    }

    #[test]
    fn test_cpp_multiple_callback_violations() {
        let code = "auto qos = rclcpp::QoS(10).best_effort();\nrclcpp::spin_until_future_complete(node, future);\n";
        let violations = lint_cpp(code);
        assert_eq!(violations.len(), 2);
    }

    #[test]
    fn test_cpp_realtime_heap_allocation() {
        let code = "auto sub = node->create_subscription<std_msgs::msg::String>(\"topic\", 10, [](auto msg) { auto data = std::make_shared<std::string>(\"alloc\"); });\n";
        let violations = lint_cpp(code);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("Real-Time Hazard"));
    }
}



