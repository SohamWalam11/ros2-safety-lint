use crate::sros2::LintViolation;
use roxmltree::Document;

/// Checks URDF and Xacro files for physical safety risks
pub fn lint_urdf(doc: &Document<'_>) -> Vec<LintViolation> {
    let mut violations = Vec::new();

    for node in doc.descendants() {
        if node.has_tag_name("joint") {
            let j_type = node.attribute("type").unwrap_or("");
            if j_type == "revolute" || j_type == "prismatic" || j_type == "continuous" {
                let has_limit = node.children().any(|c| c.has_tag_name("limit"));
                // continuous joints might not need position limits, but they absolutely need velocity/effort limits.
                // In URDF, <limit> is required for revolute and prismatic.
                if !has_limit && (j_type == "revolute" || j_type == "prismatic") {
                    violations.push(LintViolation {
                        message: format!("Physical Safety Risk: '{}' joint is missing <limit> tag. This can cause runaway physics in simulation or real hardware.", j_type),
                        range: node.range(),
                    });
                }
            }
        }

        if node.has_tag_name("link") {
            let has_visual = node.children().any(|c| c.has_tag_name("visual"));
            let has_collision = node.children().any(|c| c.has_tag_name("collision"));

            if has_visual && !has_collision {
                violations.push(LintViolation {
                    message: "Physical Safety Risk: <link> has <visual> geometry but is missing <collision> geometry. This link will pass through objects.".to_string(),
                    range: node.range(),
                });
            }
        }
    }

    violations
}
