use crate::sros2::LintViolation;
use roxmltree::Document;

/// Checks package.xml files for hygiene and ecosystem safety
pub fn lint_package_xml(doc: &Document<'_>) -> Vec<LintViolation> {
    let mut violations = Vec::new();

    // Check package format
    if let Some(package_node) = doc.descendants().find(|n| n.has_tag_name("package")) {
        let format = package_node.attribute("format").unwrap_or("1");
        if format == "1" || format == "2" {
            violations.push(LintViolation {
                message: format!(
                    "Legacy package.xml format='{}'. ROS 2 packages should use format='3'.",
                    format
                ),
                range: package_node.range(),
            });
        }
    }

    // Check for license
    let has_license = doc.descendants().any(|n| n.has_tag_name("license"));
    if !has_license {
        // Find the root package node to attach the violation to
        if let Some(package_node) = doc.descendants().find(|n| n.has_tag_name("package")) {
            violations.push(LintViolation {
                message: "Missing <license> tag. Open Source robotics packages must declare a license to prevent accidental proprietary poisoning.".to_string(),
                range: package_node.range(),
            });
        }
    }

    violations
}
