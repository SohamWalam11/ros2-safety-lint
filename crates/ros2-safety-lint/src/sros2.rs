use roxmltree::Document;

#[derive(Debug, PartialEq)]
pub struct LintViolation {
    pub message: String,
    pub range: std::ops::Range<usize>,
}

/// Checks `permissions.xml` for wildcard subjects.
pub fn lint_permissions(doc: &Document<'_>) -> Vec<LintViolation> {
    let mut violations = Vec::new();

    for node in doc.descendants() {
        if node.has_tag_name("subject_name") {
            if let Some(text) = node.text() {
                if text.trim() == "*" {
                    violations.push(LintViolation {
                        message: "Wildcard subject '*' found in permissions.xml. This is a severe security risk.".to_string(),
                        range: node.range(),
                    });
                }
            }
        }
    }

    violations
}

/// Checks `governance.xml` for insecure rtps_protection_kind.
pub fn lint_governance(doc: &Document<'_>) -> Vec<LintViolation> {
    let mut violations = Vec::new();

    for node in doc.descendants() {
        if node.has_tag_name("rtps_protection_kind") {
            if let Some(text) = node.text() {
                let text = text.trim().to_uppercase();
                if text == "NONE" || text == "SIGN" {
                    violations.push(LintViolation {
                        message: format!(
                            "Insecure rtps_protection_kind '{}' found. Expected 'ENCRYPT'.",
                            text
                        ),
                        range: node.range(),
                    });
                }
            }
        }
    }

    violations
}

/// Checks XML launch files for hardcoded absolute paths in ROS_SECURITY_KEYSTORE.
pub fn lint_keystore_paths(doc: &Document<'_>) -> Vec<LintViolation> {
    let mut violations = Vec::new();

    for node in doc.descendants() {
        if node.has_tag_name("env") || node.has_tag_name("set_env") {
            let name = node.attribute("name").unwrap_or("");
            if name == "ROS_SECURITY_KEYSTORE" {
                if let Some(value) = node.attribute("value") {
                    if value.starts_with('/') || value.starts_with("C:\\") {
                        violations.push(LintViolation {
                            message: format!("Hardcoded absolute path '{}' for ROS_SECURITY_KEYSTORE. Use relative paths or parameterization for portability.", value),
                            range: node.range(),
                        });
                    }
                }
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
    fn test_lint_permissions_wildcard() {
        let xml = "<permissions><subject_name>*</subject_name></permissions>";
        let doc = Document::parse(xml).unwrap();
        let violations = lint_permissions(&doc);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("Wildcard subject"));
    }

    #[test]
    fn test_lint_permissions_valid() {
        let xml = "<permissions><subject_name>/robot_node</subject_name></permissions>";
        let doc = Document::parse(xml).unwrap();
        let violations = lint_permissions(&doc);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_lint_governance_insecure() {
        let xml = "<governance><rtps_protection_kind>NONE</rtps_protection_kind></governance>";
        let doc = Document::parse(xml).unwrap();
        let violations = lint_governance(&doc);
        assert_eq!(violations.len(), 1);
        assert!(violations[0]
            .message
            .contains("Insecure rtps_protection_kind"));
    }

    #[test]
    fn test_lint_governance_secure() {
        let xml = "<governance><rtps_protection_kind>ENCRYPT</rtps_protection_kind></governance>";
        let doc = Document::parse(xml).unwrap();
        let violations = lint_governance(&doc);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_lint_keystore_paths_absolute() {
        let xml = "<launch><env name=\"ROS_SECURITY_KEYSTORE\" value=\"/etc/ros/keys\"/></launch>";
        let doc = Document::parse(xml).unwrap();
        let violations = lint_keystore_paths(&doc);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("Hardcoded absolute path"));
    }
}
