use crate::sros2::LintViolation;
use roxmltree::Document;

/// Checks ROS 2 Launch XML files for critical autonomy safety settings.
pub fn lint_launch_xml(doc: &Document<'_>) -> Vec<LintViolation> {
    let mut violations = Vec::new();

    // Look for <node> elements and ensure they have respawn="true"
    // In autonomous systems, nodes crashing silently without a respawn policy can lead to deadlocks.
    for node in doc.descendants().filter(|n| n.has_tag_name("node")) {
        let has_respawn =
            node.attribute("respawn") == Some("true") || node.attribute("respawn") == Some("True");

        if !has_respawn {
            let node_name = node.attribute("name").unwrap_or("unnamed_node");
            violations.push(LintViolation {
                message: format!("Launch XML Safety Violation: Node '{}' is missing `respawn=\"true\"`. If this node crashes during autonomous operation, it will not restart, potentially causing a critical system failure.", node_name),
                range: node.range(),
            });
        }
    }

    violations
}
