use crate::sros2::LintViolation;
use roxmltree::Document;

/// Checks ROS 2 launch manifests and node descriptors for Lifecycle State Machine compliance.
/// Safety-critical actuator drivers and controllers must use Managed Lifecycle Nodes
/// to prevent physical runaway before sensor drivers are configured.
pub fn lint_lifecycle(doc: &Document) -> Vec<LintViolation> {
    let mut violations = Vec::new();

    for node in doc.descendants() {
        if node.has_tag_name("node") {
            let exec = node.attribute("exec").unwrap_or("");
            let pkg = node.attribute("pkg").unwrap_or("");
            let name = node.attribute("name").unwrap_or("unnamed");

            // Check if node is an actuator driver or controller
            let is_critical_actuator = exec.contains("controller")
                || exec.contains("driver")
                || exec.contains("motor")
                || exec.contains("steering")
                || pkg.contains("control")
                || pkg.contains("driver");

            if is_critical_actuator {
                let has_lifecycle_param = node.children().any(|c| {
                    c.has_tag_name("param")
                        && c.attribute("name").map_or(false, |n| n.contains("auto_start") || n.contains("lifecycle"))
                });

                let has_lifecycle_node_type = exec.contains("lifecycle") || pkg.contains("lifecycle");

                if !has_lifecycle_param && !has_lifecycle_node_type {
                    violations.push(LintViolation {
                        message: format!(
                            "Lifecycle Safety Hazard: Safety-critical node '{}' (pkg: '{}', exec: '{}') is not configured as a Managed Lifecycle Node. Unmanaged actuator nodes can start publishing before sensors configure, causing physical runaway.",
                            name, pkg, exec
                        ),
                        range: node.range(),
                    });
                }
            }
        }
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lifecycle_unmanaged_driver() {
        let xml = "<launch><node pkg=\"motor_driver\" exec=\"motor_controller_node\" name=\"motor_node\"/></launch>";
        let doc = Document::parse(xml).unwrap();
        let violations = lint_lifecycle(&doc);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("Lifecycle Safety Hazard"));
    }

    #[test]
    fn test_lifecycle_managed_driver() {
        let xml = "<launch><node pkg=\"motor_driver_lifecycle\" exec=\"lifecycle_motor_node\" name=\"motor_node\"/></launch>";
        let doc = Document::parse(xml).unwrap();
        let violations = lint_lifecycle(&doc);
        assert_eq!(violations.len(), 0);
    }
}
