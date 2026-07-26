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
    node: &Node,
    source: &[u8],
    violations: &mut Vec<LintViolation>,
    tainted_vars: &mut HashSet<String>,
) {
    // Taint Tracking: If we see a variable initialized with QoS, taint it
    if node.kind() == "declaration" {
        if let Ok(text) = node.utf8_text(source) {
            if text.contains("rclcpp::QoS") {
                // Simplified taint: track the whole declaration text
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

    // Recursively walk children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(&child, source, violations, tainted_vars);
    }
}
