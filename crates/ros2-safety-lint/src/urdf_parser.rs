use crate::sros2::LintViolation;
use roxmltree::Document;

/// Checks URDF and Xacro files for physical safety risks
pub fn lint_urdf(doc: &Document<'_>) -> Vec<LintViolation> {
    let mut violations = Vec::new();

    for node in doc.descendants() {
        if node.has_tag_name("joint") {
            let j_type = node.attribute("type").unwrap_or("");
            if j_type == "revolute" || j_type == "prismatic" || j_type == "continuous" {
                let limit_node = node.children().find(|c| c.has_tag_name("limit"));
                if let Some(limit) = limit_node {
                    if let Some(vel_str) = limit.attribute("velocity") {
                        if let Ok(vel) = vel_str.parse::<f64>() {
                            if vel <= 0.0 || vel > 100.0 {
                                violations.push(LintViolation {
                                    message: format!("Physical Safety Risk: Joint '{}' limit velocity ({}) is invalid or dangerously excessive.", node.attribute("name").unwrap_or("unnamed"), vel),
                                    range: limit.range(),
                                });
                            }
                        }
                    }
                    if let Some(eff_str) = limit.attribute("effort") {
                        if let Ok(eff) = eff_str.parse::<f64>() {
                            if eff <= 0.0 {
                                violations.push(LintViolation {
                                    message: format!("Physical Safety Risk: Joint '{}' limit effort ({}) must be positive.", node.attribute("name").unwrap_or("unnamed"), eff),
                                    range: limit.range(),
                                });
                            }
                        }
                    }
                } else if j_type == "revolute" || j_type == "prismatic" || j_type == "continuous" {
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

#[cfg(test)]
mod tests {
    use super::*;
    use roxmltree::Document;

    #[test]
    fn test_urdf_missing_limit() {
        let xml = "<robot><joint name=\"arm_joint\" type=\"revolute\"></joint></robot>";
        let doc = Document::parse(xml).unwrap();
        let violations = lint_urdf(&doc);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("missing <limit> tag"));
    }

    #[test]
    fn test_urdf_invalid_limit_velocity() {
        let xml = "<robot><joint name=\"wheel_joint\" type=\"continuous\"><limit velocity=\"-5\" effort=\"10\"/></joint></robot>";
        let doc = Document::parse(xml).unwrap();
        let violations = lint_urdf(&doc);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("velocity (-5) is invalid"));
    }

    #[test]
    fn test_urdf_missing_collision() {
        let xml = "<robot><link name=\"base_link\"><visual><geometry><box size=\"1 1 1\"/></geometry></visual></link></robot>";
        let doc = Document::parse(xml).unwrap();
        let violations = lint_urdf(&doc);
        assert_eq!(violations.len(), 1);
        assert!(violations[0]
            .message
            .contains("missing <collision> geometry"));
    }

    #[test]
    fn test_urdf_valid() {
        let xml = "<robot><link name=\"base_link\"><visual/><collision/></link><joint name=\"j1\" type=\"revolute\"><limit effort=\"10\" velocity=\"1\"/></joint></robot>";
        let doc = Document::parse(xml).unwrap();
        let violations = lint_urdf(&doc);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_urdf_invalid_effort() {
        let xml = "<robot><joint name=\"j1\" type=\"revolute\"><limit effort=\"-10\" velocity=\"1\"/></joint></robot>";
        let doc = Document::parse(xml).unwrap();
        let violations = lint_urdf(&doc);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("limit effort (-10) must be positive"));
    }

    #[test]
    fn test_urdf_excessive_velocity() {
        let xml = "<robot><joint name=\"j1\" type=\"revolute\"><limit effort=\"10\" velocity=\"150\"/></joint></robot>";
        let doc = Document::parse(xml).unwrap();
        let violations = lint_urdf(&doc);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("velocity (150) is invalid or dangerously excessive"));
    }



    #[test]
    fn test_urdf_multiple_joint_errors() {
        let xml = "<robot><joint name=\"j1\" type=\"revolute\"></joint><joint name=\"j2\" type=\"revolute\"><limit velocity=\"-1\" effort=\"-1\"/></joint></robot>";
        let doc = Document::parse(xml).unwrap();
        let violations = lint_urdf(&doc);
        assert_eq!(violations.len(), 3);
    }
}


