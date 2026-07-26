#![allow(clippy::collapsible_match, clippy::only_used_in_recursion)]
use crate::sros2::LintViolation;
use regex::Regex;
use rustpython_parser::ast;
use rustpython_parser::{parse, Mode};

pub fn lint_python(content: &str) -> Vec<LintViolation> {
    let mut violations = Vec::new();

    // Check for hardcoded IPs in strings
    let ip_regex = Regex::new(r#"(['"])(192\.168\.\d+\.\d+|10\.\d+\.\d+\.\d+)(['"])"#).unwrap();
    if ip_regex.is_match(content) {
        violations.push(LintViolation {
            message:
                "Hardcoded local IP address found. Use ROS parameters or environment variables."
                    .to_string(),
            range: 0..1, // In a real parser we'd find the AST node
        });
    }

    // Check for sudo usage
    let sudo_regex = Regex::new(r#"(['"])sudo(['"])"#).unwrap();
    for mat in sudo_regex.find_iter(content) {
        violations.push(LintViolation {
            message: "Privilege Escalation Risk: 'sudo' detected in launch script. Running ROS 2 nodes as root is highly discouraged.".to_string(),
            range: mat.range(),
        });
    }

    // 2. AST parsing for semantic logic
    if let Ok(ast_module) = parse(content, Mode::Module, "<embedded>") {
        if let ast::Mod::Module(module) = ast_module {
            for stmt in module.body {
                walk_stmt(&stmt, &mut violations, content);
            }
        }
    }

    violations
}

fn walk_stmt(stmt: &ast::Stmt, violations: &mut Vec<LintViolation>, content: &str) {
    match stmt {
        ast::Stmt::Assign(assign) => {
            if let ast::Expr::Attribute(attr) = &*assign.value {
                if attr.attr.as_str() == "BEST_EFFORT" {
                    violations.push(LintViolation {
                        message: "QoSReliabilityPolicy.BEST_EFFORT assigned in launch file. Ensure this is only used for high-frequency sensor data.".to_string(),
                        range: 0..1,
                    });
                }
            }
        }
        ast::Stmt::Expr(expr) => {
            walk_expr(&expr.value, violations, content);
        }
        ast::Stmt::FunctionDef(func) => {
            for s in &func.body {
                walk_stmt(s, violations, content);
            }
        }
        ast::Stmt::If(if_stmt) => {
            for s in &if_stmt.body {
                walk_stmt(s, violations, content);
            }
            for s in &if_stmt.orelse {
                walk_stmt(s, violations, content);
            }
        }
        _ => {}
    }
}

fn walk_expr(expr: &ast::Expr, violations: &mut Vec<LintViolation>, content: &str) {
    match expr {
        ast::Expr::Call(call) => {
            for arg in &call.args {
                walk_expr(arg, violations, content);
            }
            for keyword in &call.keywords {
                walk_expr(&keyword.value, violations, content);
            }
        }
        ast::Expr::Attribute(attr) if attr.attr.as_str() == "BEST_EFFORT" => {
            violations.push(LintViolation {
                    message: "QoSReliabilityPolicy.BEST_EFFORT used. Ensure this is only used for high-frequency sensor data.".to_string(),
                    range: 0..1,
                });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_python_hardcoded_ip() {
        let code = "ip = '192.168.1.50'\n";
        let violations = lint_python(code);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("Hardcoded local IP"));
    }

    #[test]
    fn test_python_sudo_usage() {
        let code = "cmd = 'sudo' + ' reboot'\n";
        let violations = lint_python(code);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("sudo"));
    }

    #[test]
    fn test_python_qos_best_effort() {
        let code = "qos = QoSReliabilityPolicy.BEST_EFFORT\n";
        let violations = lint_python(code);
        assert!(!violations.is_empty());
        assert!(violations.iter().any(|v| v.message.contains("BEST_EFFORT")));
    }

    #[test]
    fn test_python_clean() {
        let code = "def launch():\n    pass\n";
        let violations = lint_python(code);
        assert_eq!(violations.len(), 0);
    }
}
