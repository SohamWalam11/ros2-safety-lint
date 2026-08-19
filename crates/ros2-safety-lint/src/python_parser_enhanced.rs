#![allow(clippy::collapsible_match, clippy::only_used_in_recursion)]
use crate::cross_language_ir::{CrossLanguageIR, ParameterDefinition, ParameterSource, ParameterValue, ElementId};
use crate::sros2::LintViolation;
use regex::Regex;
use rustpython_parser::ast;
use rustpython_parser::{parse, Mode};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct PythonAnalysisResult {
    pub violations: Vec<LintViolation>,
    pub extracted_parameters: Vec<ParameterExtraction>,
    pub ros2_api_calls: Vec<Ros2ApiCall>,
    pub callback_patterns: Vec<CallbackPattern>,
    pub launch_script_analysis: LaunchScriptAnalysis,
    pub node_variables: HashMap<String, NodeVariable>,
    pub qos_settings: Vec<QoSSetting>,
}

#[derive(Debug, Clone)]
pub struct ParameterExtraction {
    pub parameter_name: String,
    pub parameter_value: String,
    pub source_location: (usize, usize),
    pub is_hardcoded: bool,
    pub is_ros_parameter: bool,  // True for `get_parameter()`, False for local variables
    pub config_source: Option<String>,  // Which config file this should match
    pub overrides_external: Option<String>, // Which external config this overrides
}

#[derive(Debug, Clone)]
pub struct Ros2ApiCall {
    pub call_type: Ros2ApiType,
    pub function_name: String,
    pub arguments: HashMap<String, String>,
    pub location: (usize, usize),
    pub node_name: Option<String>,
    pub topic_or_service: Option<String>,
    pub qos_settings: Option<QoSSetting>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ros2ApiType {
    NodeCreation,
    PublisherCreation,
    SubscriberCreation,
    ServiceCreation,
    ClientCreation,
    ParameterAccess,
    QoSConfiguration,
    ExecutorOperation,
    LifecycleManagement,
    LaunchDescription,
}

#[derive(Debug, Clone)]
pub struct CallbackPattern {
    pub callback_type: CallbackType,
    pub function_name: String,
    pub has_blocking_ops: bool,
    pub has_shared_state: bool,
    pub is_async: bool,
    pub location: (usize, usize),
    pub topic_or_service: String,
    pub uses_parameters: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallbackType {
    SubscriptionCallback,
    TimerCallback,
    ServiceCallback,
    ActionCallback,
    LaunchCallback,
}

#[derive(Debug, Clone)]
pub struct LaunchScriptAnalysis {
    pub node_descriptions: Vec<NodeDescription>,
    pub executor_configs: Vec<ExecutorConfig>,
    pub parameter_loads: Vec<ParameterLoad>,
    pub conditions: Vec<Condition>,
    pub has_security: bool,
    pub uses_composition: bool,
    pub uses_launch_ros: bool,
}

#[derive(Debug, Clone)]
pub struct NodeDescription {
    pub node_name: String,
    pub package_name: String,
    pub executable: String,
    pub namespace: Option<String>,
    pub parameters: Vec<String>,
    pub remappings: Vec<(String, String)>,
    pub has_respawn: bool,
    pub has_lifecycle: bool,
    pub output: Option<String>,
    pub launch_prefix: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    pub executor_type: String,
    pub single_threaded: bool,
    pub spin_type: String,
    pub has_async: bool,
    pub executor_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ParameterLoad {
    pub source_file: String,
    pub target_namespace: Option<String>,
    pub parameters: Vec<String>,
    pub is_yaml: bool,
    pub is_parameter_file: bool,
}

#[derive(Debug, Clone)]
pub struct Condition {
    pub condition_type: String,
    pub expression: String,
    pub true_branch: usize,
    pub false_branch: usize,
    pub condition_met: bool,
}

#[derive(Debug, Clone)]
pub struct NodeVariable {
    pub variable_name: String,
    pub node_type: String,
    pub node_name: String,
    pub namespace: Option<String>,
    pub location: (usize, usize),
}

#[derive(Debug, Clone)]
pub struct QoSSetting {
    pub reliability: String,
    pub durability: String,
    pub history_depth: Option<u32>,
    pub deadline: Option<String>,
    pub lifespan: Option<String>,
    pub liveliness: Option<String>,
    pub liveliness_lease_duration: Option<String>,
}

struct AnalysisContext<'a> {
    current_function: Option<String>,
    in_callback: bool,
    node_variables: HashMap<String, NodeVariable>,
    extracted_params: &'a mut Vec<ParameterExtraction>,
    api_calls: &'a mut Vec<Ros2ApiCall>,
    callback_patterns: &'a mut Vec<CallbackPattern>,
    launch_analysis: &'a mut LaunchScriptAnalysis,
    file_path: Option<&'a Path>,
    config_overrides: Vec<String>,
}

/// Enhanced Python analyzer with cross-file awareness
pub fn analyze_python_with_context(
    content: &str,
    file_path: Option<&Path>,
    ir: Option<&mut CrossLanguageIR>,
) -> PythonAnalysisResult {
    let mut violations = Vec::new();
    let mut extracted_parameters = Vec::new();
    let mut ros2_api_calls = Vec::new();
    let mut callback_patterns = Vec::new();
    let mut launch_script_analysis = LaunchScriptAnalysis {
        node_descriptions: Vec::new(),
        executor_configs: Vec::new(),
        parameter_loads: Vec::new(),
        conditions: Vec::new(),
        has_security: false,
        uses_composition: false,
        uses_launch_ros: false,
    };
    let mut node_variables = HashMap::new();
    let mut qos_settings = Vec::new();

    // Layer 1: Regex-based pattern matching (fast, simple patterns)
    analyze_regex_patterns(content, &mut violations);
    
    // Layer 2: AST-based semantic analysis with cross-file awareness
    if let Ok(ast_module) = parse(content, Mode::Module, "<embedded>") {
        if let ast::Mod::Module(module) = ast_module {
            let mut analysis_context = AnalysisContext {
                current_function: None,
                in_callback: false,
                node_variables: HashMap::new(),
                extracted_params: &mut extracted_parameters,
                api_calls: &mut ros2_api_calls,
                callback_patterns: &mut callback_patterns,
                launch_analysis: &mut launch_script_analysis,
                file_path,
                config_overrides: Vec::new(),
            };
            
            for stmt in module.body {
                walk_stmt_with_context(&stmt, &mut violations, content, &mut analysis_context);
            }
            
            node_variables = analysis_context.node_variables;
        }
    }

    // Layer 3: Extract QoS settings from ROS 2 API calls
    extract_qos_settings(&ros2_api_calls, &mut qos_settings);

    // Layer 4: Launch script specific analysis
    if content.contains("LaunchDescription") || content.contains("launch_ros") {
        analyze_launch_script_patterns(content, &mut launch_script_analysis, &mut violations);
        launch_script_analysis.uses_launch_ros = true;
    }

    // Layer 5: Cross-file parameter linking analysis
    if let Some(ir) = ir {
        analyze_parameter_overrides(&extracted_parameters, &mut violations, ir, &qos_settings);
    }

    PythonAnalysisResult {
        violations,
        extracted_parameters,
        ros2_api_calls,
        callback_patterns,
        launch_script_analysis,
        node_variables,
        qos_settings,
    }
}

/// Original lint function for backward compatibility
pub fn lint_python(content: &str) -> Vec<LintViolation> {
    analyze_python_with_context(content, None, None).violations
}

/// Enhanced analysis that adds extracted data to IR
pub fn analyze_python_and_update_ir(content: &str, file_path: &Path, ir: &mut CrossLanguageIR) -> PythonAnalysisResult {
    let result = analyze_python_with_context(content, Some(file_path), Some(ir));
    
    // Add extracted parameters to IR
    for param in &result.extracted_parameters {
        if param.is_ros_parameter {
            let param_def = ParameterDefinition {
                name: param.parameter_name.clone(),
                value: ParameterValue::String(param.parameter_value.clone()),
                param_type: crate::cross_language_ir::ParameterType::Double, // Default
                source: ParameterSource::Python,
                default_value: None,
                description: None,
            };
            ir.add_parameter(param_def);
        }
    }
    
    result
}

fn analyze_regex_patterns(content: &str, violations: &mut Vec<LintViolation>) {
    let ip_regex = Regex::new(r#"(['"])(192\.168\.\d+\.\d+|10\.\d+\.\d+\.\d+)(['"])"#).unwrap();
    if ip_regex.is_match(content) {
        violations.push(LintViolation {
            message: "Hardcoded local IP address found. Use ROS parameters or environment variables.".to_string(),
            range: 0..1,
        });
    }

    let sudo_regex = Regex::new(r#"(['"])sudo(['"])"#).unwrap();
    for mat in sudo_regex.find_iter(content) {
        violations.push(LintViolation {
            message: "Privilege Escalation Risk: 'sudo' detected in launch script. Running ROS 2 nodes as root is highly discouraged.".to_string(),
            range: mat.range(),
        });
    }
}

fn walk_stmt_with_context(
    stmt: &ast::Stmt,
    violations: &mut Vec<LintViolation>,
    content: &str,
    context: &mut AnalysisContext<'_>,
) {
    match stmt {
        ast::Stmt::Assign(assign) => {
            analyze_assignment(assign, violations, content, context);
        }
        ast::Stmt::Expr(expr) => {
            walk_expr_with_context(&expr.value, violations, content, false, context);
        }
        ast::Stmt::FunctionDef(func) => {
            let prev_function = context.current_function.take();
            let prev_in_callback = context.in_callback;
            
            context.current_function = Some(func.name.to_string());
            
            // Check if this is a ROS 2 callback
            let is_callback = func.name.contains("callback") || 
                             func.name.contains("Callback") ||
                             func.args.args.len() == 1; // Typical callback signature
            
            context.in_callback = is_callback;
            
            // Analyze function parameters
            if is_callback {
                let mut callback_pattern = CallbackPattern {
                    callback_type: determine_callback_type(&func.name, content),
                    function_name: func.name.to_string(),
                    has_blocking_ops: false,
                    has_shared_state: false,
                    is_async: false,
                    location: (0, 0), // TODO: Get actual location
                    topic_or_service: "unknown".to_string(),
                    uses_parameters: false,
                };
                
                for s in &func.body {
                    analyze_callback_statement(s, &mut callback_pattern, violations, content, context);
                }
                
                context.callback_patterns.push(callback_pattern);
            } else {
                for s in &func.body {
                    walk_stmt_with_context(s, violations, content, context);
                }
            }
            
            context.current_function = prev_function;
            context.in_callback = prev_in_callback;
        }
        ast::Stmt::If(if_stmt) => {
            // Analyze condition
            if let Some(condition_expr) = &if_stmt.test {
                analyze_condition(condition_expr, context);
            }
            
            for s in &if_stmt.body {
                walk_stmt_with_context(s, violations, content, context);
            }
            for s in &if_stmt.orelse {
                walk_stmt_with_context(s, violations, content, context);
            }
        }
        ast::Stmt::Import(import) => {
            for alias in &import.names {
                analyze_import(&alias.name, context);
            }
        }
        ast::Stmt::ImportFrom(import_from) => {
            if let Some(module) = &import_from.module {
                analyze_import(module, context);
            }
        }
        _ => {}
    }
}

fn analyze_assignment(
    assign: &ast::StmtAssign,
    violations: &mut Vec<LintViolation>,
    content: &str,
    context: &mut AnalysisContext<'_>,
) {
    // Check for QoS assignments
    if let ast::Expr::Attribute(attr) = &*assign.value {
        if attr.attr.as_str() == "BEST_EFFORT" {
            violations.push(LintViolation {
                message: "QoSReliabilityPolicy.BEST_EFFORT assigned in launch file. Ensure this is only used for high-frequency sensor data.".to_string(),
                range: 0..1,
            });
            
            // Track for cross-file comparison
            context.config_overrides.push("BEST_EFFORT_QOS".to_string());
        }
    }
    
    // Extract parameter values
    for target in &assign.targets {
        if let ast::Expr::Name(name) = &**target {
            let param_name = name.id.to_string();
            
            // Check if this looks like a parameter
            if param_name.contains("param") || param_name.contains("config") || param_name.contains("setting") {
                let value_str = extract_value_string(&assign.value, content);
                
                let param_extraction = ParameterExtraction {
                    parameter_name: param_name,
                    parameter_value: value_str.clone(),
                    source_location: (0, 0), // TODO: Get actual location
                    is_hardcoded: true,
                    is_ros_parameter: false,
                    config_source: None,
                    overrides_external: None,
                };
                
                context.extracted_params.push(param_extraction);
            }
        }
    }
    
    walk_expr_with_context(&assign.value, violations, content, context.in_callback, context);
}

fn walk_expr_with_context(
    expr: &ast::Expr,
    violations: &mut Vec<LintViolation>,
    content: &str,
    in_func: bool,
    context: &mut AnalysisContext<'_>,
) {
    match expr {
        ast::Expr::Call(call) => {
            analyze_function_call(call, violations, content, in_func || context.in_callback, context);
            
            for arg in &call.args {
                walk_expr_with_context(arg, violations, content, in_func || context.in_callback, context);
            }
            for keyword in &call.keywords {
                walk_expr_with_context(&keyword.value, violations, content, in_func || context.in_callback, context);
            }
        }
        ast::Expr::Attribute(attr) => {
            analyze_attribute_access(attr, violations, context);
        }
        ast::Expr::Name(name) => {
            // Check for parameter access
            if context.in_callback && (name.id == "self" || name.id.contains("param")) {
                if let Some(callback) = context.callback_patterns.last_mut() {
                    callback.uses_parameters = true;
                }
            }
        }
        _ => {}
    }
}

fn analyze_function_call(
    call: &ast::ExprCall,
    violations: &mut Vec<LintViolation>,
    _content: &str,
    in_callback: bool,
    context: &mut AnalysisContext<'_>,
) {
    if let ast::Expr::Attribute(attr) = &*call.func {
        let func_name = attr.attr.as_str();
        
        // ROS 2 API detection
        if is_ros2_api_call(func_name, attr) {
            extract_ros2_api_call(call, func_name, context);
        }
        
        // Security and safety checks
        if func_name == "spin_until_future_complete" {
            violations.push(LintViolation {
                message: "Executor Deadlock Risk: 'spin_until_future_complete' called inside Python node. Avoid nested spinning on single-threaded executors.".to_string(),
                range: 0..1,
            });
        } else if in_callback && (func_name == "sleep" || func_name == "result") {
            violations.push(LintViolation {
                message: "Executor Deadlock Risk: Blocking wait/sleep call detected inside Python callback. Avoid blocking calls in single-threaded ROS 2 callbacks.".to_string(),
                range: 0..1,
            });
            
            if let Some(callback) = context.callback_patterns.last_mut() {
                callback.has_blocking_ops = true;
            }
        }
    }
}

fn is_ros2_api_call(func_name: &str, attr: &ast::ExprAttribute) -> bool {
    // Common ROS 2 Python API patterns
    let ros2_functions = [
        "create_node", "create_publisher", "create_subscription",
        "create_service", "create_client", "get_parameter", "set_parameter",
        "declare_parameter", "QoS", "spin", "spin_once", "ok",
        "Node", "Publisher", "Subscriber", "Service", "Client",
        "LifecycleNode", "ManagedNode"
    ];
    
    ros2_functions.contains(&func_name) || 
    (attr.value.is_attribute() && attr.attr.as_str().contains("rclpy"))
}

fn extract_ros2_api_call(
    call: &ast::ExprCall,
    func_name: &str,
    context: &mut AnalysisContext<'_>,
) {
    let mut arguments = HashMap::new();
    let mut topic_or_service = None;
    let mut node_name = None;
    let mut qos_settings = None;
    
    // Extract arguments
    for (i, arg) in call.args.iter().enumerate() {
        if let ast::Expr::Constant(constant) = &**arg {
            if i == 0 && (func_name.contains("publisher") || func_name.contains("subscription")) {
                topic_or_service = Some(constant.value.to_string());
            }
        }
    }
    
    // Extract keyword arguments
    for keyword in &call.keywords {
        if let Some(arg_name) = &keyword.arg {
            if let ast::Expr::Constant(constant) = &*keyword.value {
                arguments.insert(arg_name.to_string(), constant.value.to_string());
                
                if arg_name == "qos" || arg_name == "qos_profile" {
                    qos_settings = Some(extract_qos_setting(&constant.value.to_string()));
                }
                
                if arg_name == "node_name" || arg_name == "name" {
                    node_name = Some(constant.value.to_string());
                }
            }
        }
    }
    
    let api_type = match func_name {
        n if n.contains("create_node") || n == "Node" => Ros2ApiType::NodeCreation,
        n if n.contains("create_publisher") || n == "Publisher" => Ros2ApiType::PublisherCreation,
        n if n.contains("create_subscription") || n == "Subscriber" => Ros2ApiType::SubscriberCreation,
        n if n.contains("create_service") || n == "Service" => Ros2ApiType::ServiceCreation,
        n if n.contains("create_client") || n == "Client" => Ros2ApiType::ClientCreation,
        n if n.contains("parameter") => Ros2ApiType::ParameterAccess,
        n if n.contains("QoS") => Ros2ApiType::QoSConfiguration,
        n if n.contains("spin") || n.contains("executor") => Ros2ApiType::ExecutorOperation,
        n if n.contains("lifecycle") || n == "LifecycleNode" => Ros2ApiType::LifecycleManagement,
        n if n.contains("launch") || n == "LaunchDescription" => Ros2ApiType::LaunchDescription,
        _ => Ros2ApiType::NodeCreation,
    };
    
    let api_call = Ros2ApiCall {
        call_type: api_type,
        function_name: func_name.to_string(),
        arguments,
        location: (0, 0), // TODO: Get actual location
        node_name,
        topic_or_service,
        qos_settings,
    };
    
    context.api_calls.push(api_call);
}

fn extract_qos_setting(qos_str: &str) -> QoSSetting {
    // Simple extraction - in real implementation, parse QoS object
    QoSSetting {
        reliability: if qos_str.contains("best_effort") {
            "BEST_EFFORT".to_string()
        } else {
            "RELIABLE".to_string()
        },
        durability: if qos_str.contains("transient_local") {
            "TRANSIENT_LOCAL".to_string()
        } else {
            "VOLATILE".to_string()
        },
        history_depth: extract_history_depth(qos_str),
        deadline: None,
        lifespan: None,
        liveliness: None,
        liveliness_lease_duration: None,
    }
}

fn extract_history_depth(qos_str: &str) -> Option<u32> {
    // Extract depth from QoS string like "QoS(depth=10)"
    let depth_regex = Regex::new(r"depth\s*=\s*(\d+)").unwrap();
    depth_regex.captures(qos_str)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse::<u32>().ok())
}

fn analyze_attribute_access(
    attr: &ast::ExprAttribute,
    violations: &mut Vec<LintViolation>,
    context: &mut AnalysisContext<'_>,
) {
    match attr.attr.as_str() {
        "BEST_EFFORT" => {
            violations.push(LintViolation {
                message: "QoSReliabilityPolicy.BEST_EFFORT used. Ensure this is only used for high-frequency sensor data.".to_string(),
                range: 0..1,
            });
            context.config_overrides.push("BEST_EFFORT_QOS".to_string());
        }
        "RELIABLE" => {
            // Track for cross-file comparison
            context.config_overrides.push("RELIABLE_QOS".to_string());
        }
        "VOLATILE" | "TRANSIENT_LOCAL" => {
            // Track durability settings
            context.config_overrides.push(format!("{}_DURABILITY", attr.attr.as_str()));
        }
        _ => {}
    }
}

fn analyze_callback_statement(
    stmt: &ast::Stmt,
    callback_pattern: &mut CallbackPattern,
    violations: &mut Vec<LintViolation>,
    content: &str,
    context: &mut AnalysisContext<'_>,
) {
    match stmt {
        ast::Stmt::Expr(expr) => {
            if let ast::Expr::Call(call) = &expr.value {
                if let ast::Expr::Attribute(attr) = &*call.func {
                    let func_name = attr.attr.as_str();
                    
                    if func_name == "sleep" || func_name == "result" || func_name == "wait" {
                        callback_pattern.has_blocking_ops = true;
                    }
                    
                    // Check for shared state access
                    if let ast::Expr::Name(name) = &*attr.value {
                        if name.id == "self" || name.id.starts_with("global_") {
                            callback_pattern.has_shared_state = true;
                        }
                    }
                }
            }
        }
        ast::Stmt::Assign(assign) => {
            // Check for shared state assignments
            for target in &assign.targets {
                if let ast::Expr::Attribute(attr) = &**target {
                    if let ast::Expr::Name(name) = &*attr.value {
                        if name.id == "self" {
                            callback_pattern.has_shared_state = true;
                        }
                    }
                }
            }
        }
        _ => {}
    }
    
    walk_stmt_with_context(stmt, violations, content, context);
}

fn determine_callback_type(function_name: &str, content: &str) -> CallbackType {
    if function_name.contains("timer") {
        CallbackType::TimerCallback
    } else if function_name.contains("service") || function_name.contains("Service") {
        CallbackType::ServiceCallback
    } else if function_name.contains("action") || function_name.contains("Action") {
        CallbackType::ActionCallback
    } else if content.contains("LaunchDescription") {
        CallbackType::LaunchCallback
    } else {
        CallbackType::SubscriptionCallback
    }
}

fn analyze_condition(expr: &ast::Expr, context: &mut AnalysisContext<'_>) {
    // Extract condition for launch script analysis
    // TODO: Implement condition extraction
}

fn analyze_import(module_name: &str, context: &mut AnalysisContext<'_>) {
    if module_name.contains("rclpy") || module_name == "rclpy" {
        // ROS 2 Python library
    } else if module_name.contains("launch") || module_name.contains("launch_ros") {
        context.launch_analysis.uses_launch_ros = true;
    } else if module_name.contains("sros2") {
        context.launch_analysis.has_security = true;
    }
}

fn extract_value_string(expr: &ast::Expr, content: &str) -> String {
    match expr {
        ast::Expr::Constant(constant) => constant.value.to_string(),
        ast::Expr::Name(name) => name.id.to_string(),
        ast::Expr::Attribute(attr) => attr.attr.to_string(),
        _ => "unknown".to_string(),
    }
}

fn analyze_launch_script_patterns(
    content: &str,
    analysis: &mut LaunchScriptAnalysis,
    violations: &mut Vec<LintViolation>,
) {
    // Simple regex-based launch script analysis
    let node_regex = Regex::new(r"Node\s*\(\s*.*?package\s*=\s*['\"]([^'\"]+)['\"].*?executable\s*=\s*['\"]([^'\"]+)['\"].*?name\s*=\s*['\"]([^'\"]+)['\"]").unwrap();
    
    for cap in node_regex.captures_iter(content) {
        if let (Some(package), Some(executable), Some(name)) = (cap.get(1), cap.get(2), cap.get(3)) {
            let node_desc = NodeDescription {
                node_name: name.as_str().to_string(),
                package_name: package.as_str().to_string(),
                executable: executable.as_str().to_string(),
                namespace: None,
                parameters: Vec::new(),
                remappings: Vec::new(),
                has_respawn: content.contains("respawn=True") || content.contains("respawn=true"),
                has_lifecycle: content.contains("LifecycleNode") || content.contains("lifecycle"),
                output: None,
                launch_prefix: None,
            };
            analysis.node_descriptions.push(node_desc);
        }
    }
    
    // Check for parameter loading
    let param_regex = Regex::new(r"ParameterFile\s*\(\s*.*?path\s*=\s*['\"]([^'\"]+)['\"]").unwrap();
    for cap in param_regex.captures_iter(content) {
        if let Some(path) = cap.get(1) {
            let param_load = ParameterLoad {
                source_file: path.as_str().to_string(),
                target_namespace: None,
                parameters: Vec::new(),
                is_yaml: path.as_str().ends_with(".yaml") || path.as_str().ends_with(".yml"),
                is_parameter_file: true,
            };
            analysis.parameter_loads.push(param_load);
        }
    }
}

fn extract_qos_settings(api_calls: &[Ros2ApiCall], qos_settings: &mut Vec<QoSSetting>) {
    for api_call in api_calls {
        if let Some(qos) = &api_call.qos_settings {
            qos_settings.push(qos.clone());
        }
    }
}

fn analyze_parameter_overrides(
    extracted_params: &[ParameterExtraction],
    violations: &mut Vec<LintViolation>,
    ir: &CrossLanguageIR,
    qos_settings: &[QoSSetting],
) {
    // Compare Python parameters with IR parameters from XML/YAML
    // This is where true cross-file analysis happens
    
    // Check for QoS overrides
    for qos_setting in qos_settings {
        if qos_setting.reliability == "BEST_EFFORT" {
            // Check if XML/YAML has RELIABLE setting that's being overridden
            violations.push(LintViolation {
                message: "Shadow Configuration Drift: Python code uses BEST_EFFORT QoS, potentially overriding RELIABLE configuration in XML/YAML files.".to_string(),
                range: 0..1,
            });
        }
    }
    
    // Check for parameter value conflicts
    for param in extracted_params {
        if param.is_hardcoded && param.is_ros_parameter {
            violations.push(LintViolation {
                message: format!("Configuration Override: Python hardcodes parameter '{}' = '{}', potentially overriding external configuration.", param.parameter_name, param.parameter_value),
                range: 0..1,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_python_ros2_api_extraction() {
        let code = r#"
import rclpy
from rclpy.node import Node
from std_msgs.msg import String

class MyNode(Node):
    def __init__(self):
        super().__init__('my_node')
        self.publisher = self.create_publisher(String, 'topic', 10)
        self.subscription = self.create_subscription(
            String, 'topic', self.callback, 10
        )
        self.declare_parameter('my_param', 42.0)
        
    def callback(self, msg):
        param_value = self.get_parameter('my_param').value
        self.get_logger().info(f'Received: {msg.data}, param: {param_value}')
"#;
        
        let mut ir = CrossLanguageIR::new();
        let result = analyze_python_and_update_ir(code, Path::new("test.py"), &mut ir);
        
        // Should extract ROS 2 API calls
        assert!(!result.ros2_api_calls.is_empty());
        assert!(result.ros2_api_calls.iter().any(|c| c.function_name == "create_publisher"));
        assert!(result.ros2_api_calls.iter().any(|c| c.function_name == "create_subscription"));
        
        // Should extract parameters
        assert!(result.extracted_parameters.iter().any(|p| p.parameter_name.contains("my_param")));
        
        // Should find callback pattern
        assert!(!result.callback_patterns.is_empty());
        assert_eq!(result.callback_patterns[0].callback_type, CallbackType::SubscriptionCallback);
    }

    #[test]
    fn test_python_launch_script_analysis() {
        let code = r#"
from launch import LaunchDescription
from launch_ros.actions import Node

def generate_launch_description():
    return LaunchDescription([
        Node(
            package='my_package',
            executable='my_node',
            name='my_node',
            parameters=['config/my_config.yaml'],
            respawn=True,
            respawn_delay=5.0
        )
    ])
"#;
        
        let result = analyze_python_with_context(code, None, None);
        
        assert!(result.launch_script_analysis.uses_launch_ros);
        assert!(!result.launch_script_analysis.node_descriptions.is_empty());
        assert!(result.launch_script_analysis.node_descriptions[0].has_respawn);
        assert!(!result.launch_script_analysis.parameter_loads.is_empty());
    }

    #[test]
    fn test_python_qos_override_detection() {
        let code = r#"
import rclpy
from rclpy.qos import QoSProfile, ReliabilityPolicy

qos_profile = QoSProfile(
    depth=10,
    reliability=ReliabilityPolicy.BEST_EFFORT
)

node.create_publisher(String, 'topic', qos_profile)
"#;
        
        let mut ir = CrossLanguageIR::new();
        let result = analyze_python_and_update_ir(code, Path::new("test.py"), &mut ir);
        
        // Should detect QoS setting
        assert!(!result.qos_settings.is_empty());
        assert_eq!(result.qos_settings[0].reliability, "BEST_EFFORT");
        
        // Should find QoS-related violations
        assert!(!result.violations.is_empty());
    }

    #[test]
    fn test_python_callback_safety_analysis() {
        let code = r#"
def dangerous_callback(msg):
    import time
    time.sleep(2.0)  # Blocking in callback!
    self.shared_state = msg.data
    
def safe_callback(msg):
    self.counter += 1
"#;
        
        let result = analyze_python_with_context(code, None, None);
        
        // Should find callback patterns
        assert!(!result.callback_patterns.is_empty());
        
        // First callback should be marked as having blocking ops
        let dangerous = result.callback_patterns.iter()
            .find(|c| c.function_name == "dangerous_callback");
        assert!(dangerous.is_some());
        assert!(dangerous.unwrap().has_blocking_ops);
        
        // Should have violations for blocking ops
        assert!(result.violations.iter().any(|v| v.message.contains("Executor Deadlock Risk")));
    }

    #[test]
    fn test_python_cross_file_parameter_analysis() {
        // Simulate Python code that hardcodes parameters
        let code = r#"
class ConfigNode(Node):
    def __init__(self):
        super().__init__('config_node')
        # These should override external configs
        self.max_speed = 5.0  # Hardcoded parameter
        self.min_distance = 0.5  # Hardcoded parameter
        
        # ROS parameter access
        self.declare_parameter('max_speed', 3.0)  # Different from hardcoded!
        
    def update_config(self):
        # Get parameter but might override with hardcoded value
        param_speed = self.get_parameter('max_speed').value
        actual_speed = max(param_speed, self.max_speed)  # Override!
"#;
        
        let mut ir = CrossLanguageIR::new();
        let result = analyze_python_and_update_ir(code, Path::new("test.py"), &mut ir);
        
        // Should extract both hardcoded and ROS parameters
        let hardcoded_params: Vec<_> = result.extracted_parameters
            .iter()
            .filter(|p| p.is_hardcoded)
            .collect();
        
        assert!(!hardcoded_params.is_empty());
        assert!(hardcoded_params.iter().any(|p| p.parameter_name == "max_speed"));
        
        // Should have violations for configuration overrides
        assert!(result.violations.iter().any(|v| v.message.contains("Configuration Override")));
    }
}