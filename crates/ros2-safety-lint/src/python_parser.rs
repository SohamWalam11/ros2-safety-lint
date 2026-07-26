#![allow(clippy::invalid_regex, clippy::collapsible_match, clippy::only_used_in_recursion)]
use crate::sros2::LintViolation;
use regex::Regex;
use rustpython_parser::ast;
use rustpython_parser::{parse, Mode};

pub fn lint_python(content: &str) -> Vec<LintViolation> {
    let mut violations = Vec::new();

    // 1. Regex checks for IPs and sudo (simulating secrets scanning)
    let ip_regex = Regex::new(r#"(['"])(192\.168\.\d+\.\d+|10\.\d+\.\d+\.\d+)\1"#).unwrap();
    for mat in ip_regex.find_iter(content) {
        violations.push(LintViolation {
            message: format!("Hardcoded Private IP address detected: {}. This is a critical security and cross-talk risk.", mat.as_str()),
            range: mat.range(),
        });
    }

    let sudo_regex = Regex::new(r#"(['"])sudo\1"#).unwrap();
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
        ast::Expr::Attribute(attr)
            if attr.attr.as_str() == "BEST_EFFORT" => {
                violations.push(LintViolation {
                    message: "QoSReliabilityPolicy.BEST_EFFORT used. Ensure this is only used for high-frequency sensor data.".to_string(),
                    range: 0..1,
                });
            }
        _ => {}
    }
}
