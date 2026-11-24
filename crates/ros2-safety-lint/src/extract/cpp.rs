//! C++ facts via tree-sitter: node classes, declared parameters, endpoints with
//! QoS, callbacks and callback groups, executors, and blocking calls.

use crate::facts::*;
use crate::source::{FileId, Loc};
use std::collections::HashMap;
use tree_sitter::{Node, Parser};

/// Quick prefilter: only parse C++ files that use ROS client APIs.
pub fn is_relevant(text: &str) -> bool {
    text.contains("rclcpp")
        // Nodes deriving from a wrapper such as `nav2_util::LifecycleNode`.
        || text.contains("LifecycleNode")
        // Component registration (checked against CMake by the packaging agent).
        || text.contains("RCLCPP_COMPONENTS_REGISTER_NODE")
        // Behavior-tree node registrations (knowledge packs, BT XML checks).
        || text.contains("BT_REGISTER_NODES")
        || text.contains("registerNodeType")
        || text.contains("declare_parameter")
        || text.contains("create_subscription")
}

#[derive(Clone)]
enum QosEvent {
    Set(QosSpec),
    Modify(String),
}

struct W<'a> {
    file: FileId,
    src: &'a str,
    facts: CodeFacts,
    /// class name → index into facts.nodes
    impls: HashMap<String, usize>,
    free_impl: Option<usize>,
    file_open: bool,
    /// future variable → client variable
    future_vars: HashMap<String, Option<String>>,
    /// options variable → callback group variable
    opt_groups: HashMap<String, String>,
    /// lambda variable → body range
    lambda_vars: HashMap<String, (usize, usize)>,
    /// QoS variable events: (var, pos, function start, event)
    qos_events: Vec<(String, usize, usize, QosEvent)>,
}

fn text<'a>(n: &Node, src: &'a str) -> &'a str {
    n.utf8_text(src.as_bytes()).unwrap_or("")
}

/// Strips `this->`, `(*this).`, a trailing `.future`, and whitespace from a variable expression.
/// Milliseconds of a C++ duration written literally: `100ms`, `1s`, `0.5s`,
/// `std::chrono::milliseconds(100)`, `std::chrono::duration<double>(0.1)`,
/// `rclcpp::Duration::from_seconds(0.1)`, optionally divided by a number (`1s / 30`).
fn duration_ms(t: &str) -> Option<f64> {
    let t: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    if let Some((a, b)) = t.rsplit_once('/') {
        let d: f64 = b.trim_end_matches(['.', 'f', 'F']).parse().ok()?;
        return (d != 0.0).then_some(duration_ms(a)? / d);
    }
    let unit_ms = |u: &str| match u {
        "h" | "hours" => Some(3_600_000.0),
        "min" | "minutes" => Some(60_000.0),
        "s" | "seconds" | "duration<double>" | "from_seconds" => Some(1000.0),
        "ms" | "milliseconds" | "duration<double,std::milli>" => Some(1.0),
        "us" | "microseconds" => Some(0.001),
        "ns" | "nanoseconds" => Some(0.000_001),
        _ => None,
    };
    let num = |s: &str| {
        s.trim_end_matches(['f', 'F'])
            .parse::<f64>()
            .ok()
            .filter(|x| *x > 0.0)
    };
    // `100ms`
    let split = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(t.len());
    if split > 0 && split < t.len() {
        if let (Some(n), Some(u)) = (num(&t[..split]), unit_ms(&t[split..])) {
            return Some(n * u);
        }
    }
    // `std::chrono::milliseconds(100)` / `{100}`
    let open = t.find(['(', '{'])?;
    let inner = t[open + 1..].strip_suffix([')', '}'])?;
    let head = &t[..open];
    let name = match head.find("duration<") {
        Some(i) => &head[i..],
        None => head.rsplit("::").next()?,
    };
    Some(num(inner)? * unit_ms(name)?)
}

fn norm_var(s: &str) -> String {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let s = s
        .trim_start_matches("this->")
        .trim_start_matches("(*this).")
        .to_string();
    s.strip_suffix(".future")
        .map(|x| x.to_string())
        .unwrap_or(s)
}

fn last_segment(s: &str) -> &str {
    let s = s.split('<').next().unwrap_or(s);
    s.rsplit("::").next().unwrap_or(s).trim()
}

/// (function name, receiver text, template args text) of a call expression.
fn call_parts<'a>(call: &Node<'a>, src: &'a str) -> (String, Option<String>, Option<String>) {
    let Some(f) = call.child_by_field_name("function") else {
        return (String::new(), None, None);
    };
    match f.kind() {
        "identifier" => (text(&f, src).to_string(), None, None),
        "field_expression" => {
            let recv = f
                .child_by_field_name("argument")
                .map(|a| text(&a, src).to_string());
            let field = f.child_by_field_name("field");
            match field {
                Some(fl) if fl.kind() == "template_method" => {
                    let name = fl
                        .child_by_field_name("name")
                        .map(|n| text(&n, src).to_string())
                        .unwrap_or_default();
                    let targs = fl
                        .child_by_field_name("arguments")
                        .map(|a| text(&a, src).to_string());
                    (name, recv, targs)
                }
                Some(fl) => (text(&fl, src).to_string(), recv, None),
                None => (String::new(), recv, None),
            }
        }
        "qualified_identifier" | "template_function" => {
            let full = text(&f, src);
            let targs = full.find('<').map(|i| full[i..].to_string());
            let name = last_segment(full).to_string();
            // Keep the namespace as a pseudo-receiver (e.g. `rclcpp`).
            let ns = full.split('<').next().unwrap_or(full);
            let recv = ns.rfind("::").map(|i| ns[..i].to_string());
            (name, recv, targs)
        }
        _ => (String::new(), None, None),
    }
}

fn args<'a>(call: &Node<'a>) -> Vec<Node<'a>> {
    let Some(al) = call.child_by_field_name("arguments") else {
        return Vec::new();
    };
    let mut c = al.walk();
    al.named_children(&mut c)
        .filter(|n| n.kind() != "comment")
        .collect()
}

fn string_value(n: &Node, src: &str) -> Option<String> {
    match n.kind() {
        "string_literal" => {
            let t = text(n, src);
            Some(
                t.trim_start_matches(|c| c != '"')
                    .trim_matches('"')
                    .to_string(),
            )
        }
        "concatenated_string" => {
            let mut c = n.walk();
            let parts: Option<Vec<String>> = n
                .named_children(&mut c)
                .map(|x| string_value(&x, src))
                .collect();
            parts.map(|p| p.concat())
        }
        _ => None,
    }
}

/// Parameter name argument → pattern. `prefix + ".x"` becomes `Suffix(".x")`.
fn param_name(n: &Node, src: &str) -> Option<NamePattern> {
    if let Some(s) = string_value(n, src) {
        return Some(NamePattern::Exact(s));
    }
    if n.kind() == "binary_expression" {
        if let Some(r) = n.child_by_field_name("right") {
            if let Some(s) = string_value(&r, src) {
                if !s.is_empty() {
                    return Some(NamePattern::Suffix(s));
                }
            }
        }
    }
    None
}

fn literal_type(n: &Node, src: &str) -> ParamType {
    let t = text(n, src).trim();
    match n.kind() {
        "number_literal" => {
            let l = t.to_ascii_lowercase();
            if l.starts_with("0x") {
                ParamType::Int
            } else if l.contains('.') || l.contains('e') || l.ends_with('f') {
                ParamType::Double
            } else {
                ParamType::Int
            }
        }
        "string_literal" | "concatenated_string" | "raw_string_literal" => ParamType::Str,
        "true" | "false" => ParamType::Bool,
        "unary_expression" => n
            .child_by_field_name("argument")
            .map_or(ParamType::Unknown, |a| literal_type(&a, src)),
        "call_expression" => {
            let (name, _, targs) = call_parts(n, src);
            if name == "ParameterValue" {
                return args(n)
                    .first()
                    .map_or(ParamType::Unknown, |a| literal_type(a, src));
            }
            if name == "string" {
                return ParamType::Str;
            }
            vector_type(targs.as_deref().unwrap_or(t))
        }
        "compound_literal_expression" | "initializer_list" => vector_type(t),
        _ => {
            if t.contains("PARAMETER_DOUBLE_ARRAY") {
                ParamType::DoubleArray
            } else if t.contains("PARAMETER_STRING_ARRAY") {
                ParamType::StrArray
            } else if t.contains("PARAMETER_INTEGER_ARRAY") {
                ParamType::IntArray
            } else if t.contains("PARAMETER_BOOL_ARRAY") {
                ParamType::BoolArray
            } else if t.contains("PARAMETER_DOUBLE") {
                ParamType::Double
            } else if t.contains("PARAMETER_INTEGER") {
                ParamType::Int
            } else if t.contains("PARAMETER_STRING") {
                ParamType::Str
            } else if t.contains("PARAMETER_BOOL") {
                ParamType::Bool
            } else if t.starts_with("std::vector<") {
                vector_type(t)
            } else {
                ParamType::Unknown
            }
        }
    }
}

fn vector_type(t: &str) -> ParamType {
    let t: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    if !t.contains("vector<") {
        return ParamType::Unknown;
    }
    if t.contains("vector<double>") || t.contains("vector<float>") {
        ParamType::DoubleArray
    } else if t.contains("vector<std::string>") || t.contains("vector<string>") {
        ParamType::StrArray
    } else if t.contains("vector<bool>") {
        ParamType::BoolArray
    } else if t.contains("vector<int") || t.contains("vector<long") || t.contains("vector<int64_t>")
    {
        ParamType::IntArray
    } else {
        ParamType::Unknown
    }
}

fn template_type(targs: &str) -> ParamType {
    let t: String = targs
        .trim_start_matches('<')
        .trim_end_matches('>')
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    match t.as_str() {
        "double" | "float" => ParamType::Double,
        "int" | "int64_t" | "long" | "int32_t" | "uint32_t" | "size_t" | "unsignedint"
        | "uint64_t" | "longint" => ParamType::Int,
        "bool" => ParamType::Bool,
        "std::string" | "string" => ParamType::Str,
        _ => vector_type(&t),
    }
}

/// Builds a QoS spec from an expression's text. Variables yield `None` (resolved by replay).
pub fn qos_from_text(t: &str) -> Option<QosSpec> {
    let c: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    let mut q = if let Ok(n) = c.parse::<u32>() {
        QosSpec {
            depth: Some(n),
            ..QosSpec::ros_default(t.trim())
        }
    } else if c.contains("SensorDataQoS") || c.contains("rmw_qos_profile_sensor_data") {
        QosSpec::sensor_data(t.trim())
    } else if let Some(custom) = custom_qos_class(&c) {
        // Project-specific QoS classes (e.g. nav2::qos::LatchedPublisherQoS): only trust the name.
        let mut q = QosSpec::unknown(t.trim());
        let l = custom.to_ascii_lowercase();
        if l.contains("latched") || l.contains("transientlocal") {
            q.durability = Some(Durability::TransientLocal);
            q.reliability = Some(Reliability::Reliable);
        }
        apply_qos_modifiers(&mut q, &c);
        return Some(q);
    } else if c.contains("SystemDefaultsQoS") || c.contains("rmw_qos_profile_system_default") {
        QosSpec::system_default(t.trim())
    } else if c.contains("QoS(")
        || c.contains("ServicesQoS")
        || c.contains("ParametersQoS")
        || c.contains("ParameterEventsQoS")
        || c.contains("rmw_qos_profile_default")
        || c.contains("rmw_qos_profile_services_default")
        || c.starts_with("rclcpp::KeepLast(")
        || c.starts_with("KeepLast(")
    {
        QosSpec::ros_default(t.trim())
    } else {
        return None;
    };
    // Depth from `QoS(5)` / `KeepLast(5)` / `.keep_last(5)`.
    for pat in ["KeepLast(", "QoS(", ".keep_last("] {
        if let Some(i) = c.rfind(pat) {
            let digits: String = c[i + pat.len()..]
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect();
            if let Ok(d) = digits.parse() {
                q.depth = Some(d);
                break;
            }
        }
    }
    apply_qos_modifiers(&mut q, &c);
    q.origin = t.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(q)
}

/// A `FooQoS(` class name that is not one of rclcpp's own profiles.
fn custom_qos_class(c: &str) -> Option<String> {
    const RCLCPP: &[&str] = &[
        "QoS",
        "SensorDataQoS",
        "SystemDefaultsQoS",
        "ServicesQoS",
        "ParametersQoS",
        "ParameterEventsQoS",
        "RosoutQoS",
        "ClockQoS",
        "BestAvailableQoS",
    ];
    let i = c.find("QoS(")?;
    let start = c[..i]
        .rfind(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .map_or(0, |p| p + 1);
    let name = &c[start..i + 3];
    if RCLCPP.contains(&name) {
        None
    } else {
        Some(name.to_string())
    }
}

/// Applies chained / statement modifiers found in `c` (whitespace-free text) in source order.
fn apply_qos_modifiers(q: &mut QosSpec, c: &str) {
    const PATTERNS: &[(&str, u8)] = &[
        (".best_effort()", 0),
        ("ReliabilityPolicy::BestEffort", 0),
        ("RMW_QOS_POLICY_RELIABILITY_BEST_EFFORT", 0),
        (".reliable()", 1),
        ("ReliabilityPolicy::Reliable", 1),
        ("RMW_QOS_POLICY_RELIABILITY_RELIABLE", 1),
        (".transient_local()", 2),
        ("DurabilityPolicy::TransientLocal", 2),
        ("RMW_QOS_POLICY_DURABILITY_TRANSIENT_LOCAL", 2),
        (".durability_volatile()", 3),
        ("DurabilityPolicy::Volatile", 3),
        ("RMW_QOS_POLICY_DURABILITY_VOLATILE", 3),
        (".deadline(", 4),
        (".liveliness(", 5),
        (".liveliness_lease_duration(", 6),
    ];
    let mut hits: Vec<(usize, u8)> = Vec::new();
    for (p, k) in PATTERNS {
        let mut from = 0;
        while let Some(i) = c[from..].find(p) {
            hits.push((from + i, *k));
            from += i + p.len();
        }
    }
    hits.sort();
    for (_, k) in hits {
        match k {
            0 => q.reliability = Some(Reliability::BestEffort),
            1 => q.reliability = Some(Reliability::Reliable),
            2 => q.durability = Some(Durability::TransientLocal),
            3 => q.durability = Some(Durability::Volatile),
            4 => q.deadline_ns = None,
            5 => q.liveliness = None,
            _ => q.lease_ns = None,
        }
    }
}

const ASYNC_FUTURE_FNS: &[&str] = &[
    "async_send_request",
    "async_get_parameters",
    "async_set_parameters",
    "async_send_goal",
    "async_get_result",
    "async_cancel_goal",
    "async_cancel_all_goals",
    "async_list_parameters",
];

impl<'a> W<'a> {
    fn loc(&self, n: &Node) -> Loc {
        Loc::new(self.file, n.start_byte(), n.end_byte())
    }

    fn impl_for(&mut self, class: Option<&str>, receiver_is_self: bool, loc: Loc) -> Option<usize> {
        match class {
            Some(c) if receiver_is_self => {
                if let Some(i) = self.impls.get(c) {
                    return Some(*i);
                }
                self.facts
                    .nodes
                    .push(new_impl(self.file, Some(c.to_string()), loc));
                let i = self.facts.nodes.len() - 1;
                self.impls.insert(c.to_string(), i);
                Some(i)
            }
            Some(c) => self.impls.get(c).copied(),
            None => {
                if let Some(i) = self.free_impl {
                    return Some(i);
                }
                self.facts.nodes.push(new_impl(self.file, None, loc));
                let i = self.facts.nodes.len() - 1;
                self.free_impl = Some(i);
                Some(i)
            }
        }
    }

    fn walk(&mut self, n: Node<'a>, class: Option<String>, func: Option<(usize, usize)>) {
        let mut class = class;
        let mut func = func;
        match n.kind() {
            "class_specifier" | "struct_specifier" => {
                if let Some(name) = n.child_by_field_name("name") {
                    let cname = last_segment(text(&name, self.src)).to_string();
                    let mut c = n.walk();
                    let base_txt = n
                        .children(&mut c)
                        .find(|ch| ch.kind() == "base_class_clause")
                        .map(|b| {
                            text(&b, self.src)
                                .trim_start_matches(':')
                                .trim()
                                .to_string()
                        });
                    if n.child_by_field_name("body").is_some()
                        && !self.facts.classes_defined.contains(&cname)
                    {
                        self.facts.classes_defined.push(cname.clone());
                    }
                    if base_txt.as_deref().is_some_and(is_node_base) {
                        let loc = self.loc(&n);
                        if let Some(i) = self.impl_for(Some(&cname), true, loc) {
                            let b = base_txt.unwrap_or_default();
                            let b = b
                                .trim_start_matches("public")
                                .trim_start_matches("protected")
                                .trim();
                            self.facts.nodes[i].base =
                                Some(b.split(',').next().unwrap_or(b).trim().to_string());
                        }
                    }
                    class = Some(cname);
                }
            }
            "function_definition" => {
                let (fname, fclass) = function_name(&n, self.src);
                if fclass.is_some() {
                    class = fclass;
                }
                if let Some(body) = n.child_by_field_name("body") {
                    func = Some((body.start_byte(), body.end_byte()));
                    self.facts.functions.push(FunctionInfo {
                        name: fname,
                        class: class.clone(),
                        start: body.start_byte(),
                        end: body.end_byte(),
                        calls: Vec::new(),
                    });
                }
            }
            "field_initializer" => self.on_field_initializer(n, class.as_deref()),
            "preproc_include" => {
                if let Some(p) = n.child_by_field_name("path") {
                    let t = text(&p, self.src).trim_matches(|c| c == '<' || c == '>' || c == '"');
                    if let Some((first, _)) = t.split_once('/') {
                        let loc = self.loc(&p);
                        self.facts.imports.push((first.to_string(), loc));
                    }
                }
            }
            "declaration" => self.on_declaration(n, func),
            "assignment_expression" => self.on_assignment(n, class.as_deref(), func),
            "call_expression" => self.on_call(n, class.as_deref(), func),
            _ => {}
        }
        let mut c = n.walk();
        let children: Vec<Node> = n.children(&mut c).collect();
        for ch in children {
            self.walk(ch, class.clone(), func);
        }
    }

    fn on_field_initializer(&mut self, n: Node<'a>, class: Option<&str>) {
        let mut c = n.walk();
        let kids: Vec<Node> = n.named_children(&mut c).collect();
        let Some(base) = kids.first() else { return };
        let bt = text(base, self.src);
        if !is_node_base(bt) {
            return;
        }
        let Some(al) = kids.iter().find(|k| k.kind() == "argument_list") else {
            return;
        };
        let mut c2 = al.walk();
        let first = al.named_children(&mut c2).next();
        let loc = self.loc(&n);
        if let Some(i) = self.impl_for(class, true, loc) {
            if let Some(s) = first.and_then(|f| string_value(&f, self.src)) {
                self.facts.nodes[i].default_name = Some(s);
            }
            self.facts.nodes[i].loc = loc;
        }
    }

    fn on_declaration(&mut self, n: Node<'a>, func: Option<(usize, usize)>) {
        let ty = n
            .child_by_field_name("type")
            .map(|t| text(&t, self.src))
            .unwrap_or("");
        let mut c = n.walk();
        for d in n.children_by_field_name("declarator", &mut c) {
            let (var, value) = match d.kind() {
                "init_declarator" => (
                    d.child_by_field_name("declarator")
                        .map(|x| norm_var(text(&x, self.src).trim_start_matches(['*', '&']))),
                    d.child_by_field_name("value"),
                ),
                _ => (Some(norm_var(text(&d, self.src))), None),
            };
            let Some(var) = var else { continue };
            // Executors declared as values: `rclcpp::executors::MultiThreadedExecutor exec;`
            if let Some(k) = executor_kind(ty) {
                let loc = self.loc(&n);
                self.facts.executors.push(ExecutorUse {
                    kind: k,
                    var: Some(var.clone()),
                    loc,
                });
            }
            // QoS objects: `rclcpp::QoS qos(10);` / `auto qos = rclcpp::QoS(10).best_effort();`
            let init_text = value.map(|v| text(&v, self.src)).unwrap_or("");
            if ty.contains("QoS")
                || init_text.contains("QoS")
                || init_text.contains("rmw_qos_profile")
            {
                let full = format!("{ty}{init_text}");
                let spec = qos_from_text(&full)
                    .or_else(|| qos_from_text(&format!("rclcpp::QoS({init_text})")));
                if let Some(s) = spec {
                    self.qos_events.push((
                        var.clone(),
                        n.start_byte(),
                        func.map_or(0, |f| f.0),
                        QosEvent::Set(s),
                    ));
                }
            }
            if let Some(v) = value {
                self.on_value_binding(&var, v, func);
            }
        }
    }

    fn on_assignment(&mut self, n: Node<'a>, _class: Option<&str>, func: Option<(usize, usize)>) {
        let (Some(l), Some(r)) = (
            n.child_by_field_name("left"),
            n.child_by_field_name("right"),
        ) else {
            return;
        };
        let lt = norm_var(text(&l, self.src));
        // options.callback_group = group_;
        if let Some(opts) = lt.strip_suffix(".callback_group") {
            self.opt_groups
                .insert(opts.to_string(), norm_var(text(&r, self.src)));
            return;
        }
        let rt = text(&r, self.src);
        if rt.contains("QoS(") || rt.contains("SensorDataQoS") || rt.contains("rmw_qos_profile") {
            if let Some(s) = qos_from_text(rt) {
                self.qos_events.push((
                    lt.clone(),
                    n.start_byte(),
                    func.map_or(0, |f| f.0),
                    QosEvent::Set(s),
                ));
            }
        }
        self.on_value_binding(&lt, r, func);
    }

    /// `var = <value>`: futures, lambdas, callback groups, executors.
    fn on_value_binding(&mut self, var: &str, v: Node<'a>, _func: Option<(usize, usize)>) {
        match v.kind() {
            "lambda_expression" => {
                if let Some(b) = v.child_by_field_name("body") {
                    self.lambda_vars
                        .insert(var.to_string(), (b.start_byte(), b.end_byte()));
                }
            }
            "call_expression" => {
                let (fname, recv, targs) = call_parts(&v, self.src);
                if ASYNC_FUTURE_FNS.contains(&fname.as_str()) {
                    self.future_vars
                        .insert(var.to_string(), recv.map(|r| norm_var(&r)));
                } else if fname == "create_callback_group" {
                    let a = args(&v);
                    let kind = if a
                        .first()
                        .is_some_and(|x| text(x, self.src).contains("Reentrant"))
                    {
                        GroupKind::Reentrant
                    } else {
                        GroupKind::MutuallyExclusive
                    };
                    let not_auto = a
                        .get(1)
                        .is_some_and(|x| text(x, self.src).trim() == "false");
                    let loc = self.loc(&v);
                    // Groups belong to whatever node class is in scope; record on all impls of this file later.
                    self.pending_group(CallbackGroupDecl {
                        var: var.to_string(),
                        kind,
                        not_auto_added: not_auto,
                        loc,
                    });
                } else if fname == "make_shared" || fname == "make_unique" {
                    if let Some(k) = targs.as_deref().and_then(executor_kind) {
                        let loc = self.loc(&v);
                        self.facts.executors.push(ExecutorUse {
                            kind: k,
                            var: Some(var.to_string()),
                            loc,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    fn pending_group(&mut self, g: CallbackGroupDecl) {
        // Attach to every node impl in the file (groups are looked up by variable name).
        if self.facts.nodes.is_empty() {
            let loc = g.loc;
            self.impl_for(None, true, loc);
        }
        for n in &mut self.facts.nodes {
            n.groups.push(g.clone());
        }
    }

    fn resolve_qos(&self, arg: &Node, func: Option<(usize, usize)>) -> QosSpec {
        let t = text(arg, self.src);
        if let Some(q) = qos_from_text(t) {
            return q;
        }
        // A variable: replay its events before this point (same function first, then file-wide).
        let var = norm_var(t);
        let pos = arg.start_byte();
        let fstart = func.map_or(usize::MAX, |f| f.0);
        let local: Vec<&(String, usize, usize, QosEvent)> = self
            .qos_events
            .iter()
            .filter(|e| e.0 == var && e.1 < pos && e.2 == fstart)
            .collect();
        let events: Vec<&(String, usize, usize, QosEvent)> =
            if local.iter().any(|e| matches!(e.3, QosEvent::Set(_))) {
                local
            } else {
                let all: Vec<_> = self.qos_events.iter().filter(|e| e.0 == var).collect();
                // A member QoS assigned in several places is ambiguous.
                if all
                    .iter()
                    .filter(|e| matches!(e.3, QosEvent::Set(_)))
                    .count()
                    != 1
                {
                    return QosSpec::unknown(t.trim());
                }
                all
            };
        let mut q: Option<QosSpec> = None;
        for (_, _, _, ev) in events {
            match ev {
                QosEvent::Set(s) => q = Some(s.clone()),
                QosEvent::Modify(m) => {
                    if let Some(q) = q.as_mut() {
                        apply_qos_modifiers(q, m);
                    }
                }
            }
        }
        q.map(|mut q| {
            q.origin = format!("{} ({})", t.trim(), q.origin);
            q
        })
        .unwrap_or_else(|| QosSpec::unknown(t.trim()))
    }

    fn callback_target(&self, a: &Node) -> Option<CallbackTarget> {
        match a.kind() {
            "lambda_expression" => a
                .child_by_field_name("body")
                .map(|b| CallbackTarget::Inline {
                    start: b.start_byte(),
                    end: b.end_byte(),
                }),
            "call_expression" => {
                let (fname, _, _) = call_parts(a, self.src);
                if fname == "bind" {
                    let first = args(a).into_iter().next()?;
                    let t = text(&first, self.src).trim_start_matches('&');
                    return Some(CallbackTarget::Function(last_segment(t).to_string()));
                }
                None
            }
            "identifier" | "field_expression" => {
                let v = norm_var(text(a, self.src));
                if let Some((s, e)) = self.lambda_vars.get(&v) {
                    return Some(CallbackTarget::Inline { start: *s, end: *e });
                }
                None
            }
            "pointer_expression" => Some(CallbackTarget::Function(
                last_segment(text(a, self.src).trim_start_matches('&')).to_string(),
            )),
            _ => None,
        }
    }

    fn on_call(&mut self, n: Node<'a>, class: Option<&str>, func: Option<(usize, usize)>) {
        let (fname, recv, targs) = call_parts(&n, self.src);
        if fname.is_empty() {
            return;
        }
        self.facts.call_sites.push((fname.clone(), n.start_byte()));
        let recv_norm = recv.as_deref().map(norm_var);
        let on_self = recv_norm
            .as_deref()
            .map_or(true, |r| r == "this" || r.is_empty());
        let a = args(&n);
        let loc = self.loc(&n);
        let full = text(&n, self.src);

        if full.contains("allow_undeclared_parameters(true)")
            || full.contains("automatically_declare_parameters_from_overrides(true)")
        {
            self.file_open = true;
        }
        if fname.contains("ClassLoader")
            || fname == "createUniqueInstance"
            || fname == "createSharedInstance"
        {
            self.facts.loads_plugins = true;
        }

        match fname.as_str() {
            // ------------------------------------------------------------ parameters
            // `add_parameter(name, default, ...)`: nav2_util::LifecycleNode's member wrapper
            // over declare_parameter (all of Nav2's AMCL parameters are declared this way).
            "declare_parameter"
            | "declare_parameter_if_not_declared"
            | "declare_or_get_parameter"
            | "add_parameter" => {
                // Free helper `f(node, name, value)` vs member `node->f(name, value)`:
                // pick the position that actually holds the name.
                let first_is_name = a.first().is_some_and(|x| param_name(x, self.src).is_some());
                let helper = fname != "declare_parameter" && !first_is_name;
                let (name_idx, val_idx) = if helper { (1, 2) } else { (0, 1) };
                let name_arg = a.get(name_idx);
                let pattern = name_arg.and_then(|x| param_name(x, self.src));
                let mut ty = targs.as_deref().map_or(ParamType::Unknown, template_type);
                let default = a.get(val_idx);
                if ty == ParamType::Unknown {
                    if let Some(d) = default {
                        ty = literal_type(d, self.src);
                    }
                }
                let has_default = default.is_some_and(|d| {
                    let t = text(d, self.src);
                    !(t.contains("ParameterType")
                        || t.starts_with("rclcpp::PARAMETER_")
                        || t.starts_with("PARAMETER_"))
                });
                let default_text = default
                    .filter(|_| has_default)
                    .map(|d| text(d, self.src).to_string());
                let target = if class.is_some()
                    && !on_self
                    && !helper
                    && !self.impls.contains_key(class.unwrap_or(""))
                {
                    // e.g. a plugin calling node->declare_parameter
                    None
                } else if helper && class.is_some() && !self.impls.contains_key(class.unwrap_or(""))
                {
                    None
                } else {
                    self.impl_for(class, on_self || helper, loc)
                };
                match (target, pattern) {
                    (Some(i), Some(p)) => self.facts.nodes[i].params.push(ParamDecl {
                        name: p,
                        ty,
                        has_default,
                        default_text,
                        loc,
                        constraints: Vec::new(),
                    }),
                    (Some(i), None) => self.facts.nodes[i].params_open = true,
                    (None, p) => self.facts.class_params.push((
                        class.map(|c| c.to_string()),
                        p.map(|name| ParamDecl {
                            name,
                            ty,
                            has_default,
                            default_text,
                            loc,
                            constraints: Vec::new(),
                        }),
                    )),
                }
            }
            "declare_parameters" => {
                if let Some(i) = self.impl_for(class, on_self, loc) {
                    self.facts.nodes[i].params_open = true;
                }
            }
            "auto_declare" => {
                // controller_interface helper: auto_declare<T>("name", default)
                if let (Some(i), Some(p)) = (
                    self.impl_for(class, true, loc),
                    a.first().and_then(|x| param_name(x, self.src)),
                ) {
                    let ty = targs.as_deref().map_or(ParamType::Unknown, template_type);
                    self.facts.nodes[i].params.push(ParamDecl {
                        name: p,
                        ty,
                        has_default: true,
                        default_text: a.get(1).map(|d| text(d, self.src).to_string()),
                        loc,
                        constraints: Vec::new(),
                    });
                }
            }
            // ------------------------------------------------------------ node creation in free functions
            "make_shared" | "Node" | "LifecycleNode"
                if class.is_none()
                    && (targs.as_deref().is_some_and(is_node_base)
                        || recv.as_deref().is_some_and(is_node_base)
                        || is_node_base(&fname)) =>
            {
                if let Some(s) = a.first().and_then(|x| string_value(x, self.src)) {
                    if let Some(i) = self.impl_for(None, true, loc) {
                        self.facts.nodes[i].default_name = Some(s);
                        self.facts.nodes[i].loc = loc;
                    }
                }
            }
            // ------------------------------------------------------------ endpoints
            "create_publisher"
            | "create_subscription"
            | "create_client"
            | "create_service"
            | "create_server"
            | "create_wall_timer"
            | "create_timer" => {
                self.on_endpoint(
                    &n,
                    &fname,
                    recv_norm.as_deref(),
                    on_self,
                    targs.as_deref(),
                    &a,
                    class,
                    func,
                    loc,
                );
            }
            "add_callback_group" if !on_self => {
                if let (Some(g), Some(i)) = (a.first(), self.facts.nodes.len().checked_sub(1)) {
                    let g = norm_var(text(g, self.src));
                    let _ = i;
                    for nimpl in &mut self.facts.nodes {
                        nimpl.dedicated_groups.push(g.clone());
                    }
                }
            }
            // ------------------------------------------------------------ executors / blocking
            "spin"
            | "spin_some"
            | "spin_all"
            | "spin_once"
            | "spin_until_future_complete"
            | "spin_node_until_future_complete" => {
                let r = recv_norm.clone().unwrap_or_default();
                let executor = if r == "rclcpp" || r.is_empty() {
                    None
                } else {
                    Some(r.clone())
                };
                if fname == "spin" && executor.is_none() {
                    self.facts.executors.push(ExecutorUse {
                        kind: ExecutorKind::SingleThreaded,
                        var: None,
                        loc,
                    });
                }
                if fname != "spin" {
                    self.facts.blocking.push(BlockingCall {
                        kind: BlockingKind::Spin { executor },
                        loc,
                    });
                }
            }
            "get" | "wait" | "wait_for" | "wait_until" => {
                if let Some(r) = recv_norm.as_deref() {
                    if let Some(client) = self.future_vars.get(r) {
                        self.facts.blocking.push(BlockingCall {
                            kind: BlockingKind::FutureWait {
                                client: client.clone(),
                            },
                            loc,
                        });
                    } else if let Some(inner) = ASYNC_FUTURE_FNS
                        .iter()
                        .find(|f| r.contains(&format!("{f}(")))
                    {
                        let client = r
                            .split(&format!("->{inner}("))
                            .next()
                            .or_else(|| r.split(&format!(".{inner}(")).next())
                            .map(norm_var);
                        self.facts.blocking.push(BlockingCall {
                            kind: BlockingKind::FutureWait { client },
                            loc,
                        });
                    }
                }
            }
            "sleep_for" | "sleep_until" | "usleep" | "nanosleep" => {
                self.facts.blocking.push(BlockingCall {
                    kind: BlockingKind::Sleep,
                    loc,
                });
            }
            "sleep"
                if recv_norm
                    .as_deref()
                    .is_some_and(|r| r.to_ascii_lowercase().contains("rate"))
                    || recv.is_none() =>
            {
                self.facts.blocking.push(BlockingCall {
                    kind: BlockingKind::Sleep,
                    loc,
                });
            }
            // QoS statement modifiers: `qos.best_effort();`
            "best_effort"
            | "reliable"
            | "transient_local"
            | "durability_volatile"
            | "reliability"
            | "durability"
            | "deadline"
            | "liveliness"
            | "liveliness_lease_duration" => {
                if let Some(r) = recv_norm.as_deref() {
                    if self.qos_events.iter().any(|e| e.0 == r) {
                        let m: String = full.chars().filter(|c| !c.is_whitespace()).collect();
                        let m = m.trim_start_matches(r).to_string();
                        self.qos_events.push((
                            r.to_string(),
                            n.start_byte(),
                            func.map_or(0, |f| f.0),
                            QosEvent::Modify(m),
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn on_endpoint(
        &mut self,
        n: &Node<'a>,
        fname: &str,
        recv: Option<&str>,
        on_self: bool,
        targs: Option<&str>,
        a: &[Node<'a>],
        class: Option<&str>,
        func: Option<(usize, usize)>,
        loc: Loc,
    ) {
        let is_action = recv.is_some_and(|r| r.contains("rclcpp_action"));
        // rclcpp_action::create_server/create_client(node, name, ...) and free create_timer(node, clock, ...)
        let skip_node_arg = is_action
            || (fname == "create_timer"
                && recv.is_some_and(|r| r == "rclcpp" || r.contains("nav2")));
        let a: Vec<Node> = if skip_node_arg {
            a.iter().skip(1).copied().collect()
        } else {
            a.to_vec()
        };
        let msg_type = targs.map(|t| {
            t.trim_start_matches('<')
                .trim_end_matches('>')
                .trim()
                .to_string()
        });
        let var = n.parent().and_then(|p| match p.kind() {
            "assignment_expression" => p
                .child_by_field_name("left")
                .map(|l| norm_var(text(&l, self.src))),
            "init_declarator" => p
                .child_by_field_name("declarator")
                .map(|d| norm_var(text(&d, self.src))),
            _ => None,
        });
        let group_of = |w: &Self, x: Option<&Node>| -> Option<String> {
            let x = x?;
            let t = norm_var(text(x, w.src));
            if t.contains("callback_group")
                || t.contains("group")
                || t.contains("cb_")
                || w.facts
                    .nodes
                    .iter()
                    .any(|n| n.groups.iter().any(|g| g.var == t))
            {
                Some(t)
            } else {
                None
            }
        };
        let impl_idx =
            if class.is_some() && !on_self && !self.impls.contains_key(class.unwrap_or("")) {
                None
            } else {
                self.impl_for(class, on_self || class.is_none(), loc)
            };
        let Some(i) = impl_idx else { return };

        match fname {
            "create_publisher" | "create_subscription" => {
                let is_sub = fname == "create_subscription";
                let name = a.first().and_then(|x| string_value(x, self.src));
                let qos = a
                    .get(1)
                    .map(|q| self.resolve_qos(q, func))
                    .unwrap_or_else(|| QosSpec::unknown(""));
                let opts_idx = if is_sub { 3 } else { 2 };
                let opts_text = a.get(opts_idx).map(|o| text(o, self.src)).unwrap_or("");
                let overrides = opts_text.contains("qos_overriding_options")
                    || opts_text.contains("QosOverridingOptions")
                    || a.get(opts_idx)
                        .is_some_and(|o| self.opts_have_overrides(text(o, self.src)));
                let (callback, group) = if is_sub {
                    let cb = a.get(2).and_then(|x| self.callback_target(x));
                    let g = a
                        .get(3)
                        .and_then(|o| self.opt_groups.get(&norm_var(text(o, self.src))).cloned());
                    (cb, g)
                } else {
                    (None, None)
                };
                if let (true, Some(cb)) = (is_sub, callback.clone()) {
                    self.facts.nodes[i].callbacks.push(Callback {
                        kind: CallbackKind::Subscription,
                        target: cb,
                        group: group.clone(),
                        period_ms: None,
                        loc,
                    });
                }
                self.facts.nodes[i].endpoints.push(Endpoint {
                    kind: if is_sub {
                        EndpointKind::Subscription
                    } else {
                        EndpointKind::Publisher
                    },
                    name,
                    msg_type,
                    qos,
                    qos_overrides_enabled: overrides,
                    var,
                    group,
                    callback,
                    loc,
                });
            }
            "create_client" => {
                let name = a.first().and_then(|x| string_value(x, self.src));
                let group = a.iter().skip(1).find_map(|x| group_of(self, Some(x)));
                self.facts.nodes[i].endpoints.push(Endpoint {
                    kind: if is_action {
                        EndpointKind::ActionClient
                    } else {
                        EndpointKind::ServiceClient
                    },
                    name,
                    msg_type,
                    qos: QosSpec::ros_default("services default"),
                    qos_overrides_enabled: false,
                    var,
                    group,
                    callback: None,
                    loc,
                });
            }
            "create_service" | "create_server" => {
                let name = a.first().and_then(|x| string_value(x, self.src));
                let cbs: Vec<CallbackTarget> = a
                    .iter()
                    .skip(1)
                    .filter_map(|x| self.callback_target(x))
                    .collect();
                let group = a.iter().skip(2).find_map(|x| group_of(self, Some(x)));
                for cb in &cbs {
                    self.facts.nodes[i].callbacks.push(Callback {
                        kind: if is_action {
                            CallbackKind::Action
                        } else {
                            CallbackKind::Service
                        },
                        target: cb.clone(),
                        group: group.clone(),
                        period_ms: None,
                        loc,
                    });
                }
                self.facts.nodes[i].endpoints.push(Endpoint {
                    kind: if is_action {
                        EndpointKind::ActionServer
                    } else {
                        EndpointKind::ServiceServer
                    },
                    name,
                    msg_type,
                    qos: QosSpec::ros_default("services default"),
                    qos_overrides_enabled: false,
                    var,
                    group,
                    callback: cbs.into_iter().next(),
                    loc,
                });
            }
            _ => {
                // timers: (period, callback, group?) — for free create_timer the clock precedes period.
                let cb_pos = a.iter().position(|x| self.callback_target(x).is_some());
                if let Some(p) = cb_pos {
                    let cb = self.callback_target(&a[p]).unwrap();
                    let group = group_of(self, a.get(p + 1));
                    self.facts.nodes[i].callbacks.push(Callback {
                        kind: CallbackKind::Timer,
                        target: cb,
                        group,
                        period_ms: p
                            .checked_sub(1)
                            .and_then(|k| a.get(k))
                            .and_then(|x| duration_ms(text(x, self.src))),
                        loc,
                    });
                }
            }
        }
    }

    fn opts_have_overrides(&self, opts_var: &str) -> bool {
        let v = norm_var(opts_var);
        // `options.qos_overriding_options = ...` anywhere in the file
        self.src.contains(&format!("{v}.qos_overriding_options"))
    }
}

fn new_impl(file: FileId, class: Option<String>, loc: Loc) -> NodeImpl {
    NodeImpl {
        file,
        lang: Lang::Cpp,
        class_name: class,
        base: None,
        default_name: None,
        params: Vec::new(),
        params_open: false,
        endpoints: Vec::new(),
        callbacks: Vec::new(),
        groups: Vec::new(),
        dedicated_groups: Vec::new(),
        loc,
    }
}

/// Ranges attached through a `ParameterDescriptor`:
/// `r.from_value = 0.0; r.to_value = 1.0; d.floating_point_range.push_back(r);` (or
/// `d.floating_point_range[0].from_value = …`) and then `declare_parameter("name", …, d)`.
/// Only fully linked, literal ranges are returned.
fn descriptor_ranges(src: &str) -> Vec<(String, Constraint)> {
    use regex::Regex;
    static RE: std::sync::OnceLock<[Regex; 3]> = std::sync::OnceLock::new();
    let [bound, push, decl] = RE.get_or_init(|| {
        [
            Regex::new(r"([A-Za-z_][\w.\[\]]*?)\.(from_value|to_value)\s*=\s*([-+]?[0-9.]+(?:[eE][-+]?[0-9]+)?)").expect("re"),
            Regex::new(r"([A-Za-z_]\w*)\.(?:floating_point_range|integer_range)\s*(?:\.push_back\(\s*|=\s*\{\s*)([A-Za-z_]\w*)").expect("re"),
            Regex::new(r#"declare_parameter\s*(?:<[^;()]*>)?\s*\(\s*"([^"]+)"[^;]*?,\s*([A-Za-z_]\w*)\s*\)\s*;"#).expect("re"),
        ]
    });
    // range variable (or `d.floating_point_range[0]`) -> (from, to)
    let mut ranges: std::collections::HashMap<String, (Option<f64>, Option<f64>)> =
        std::collections::HashMap::new();
    for c in bound.captures_iter(src) {
        let e = ranges.entry(c[1].to_string()).or_default();
        let v = c[3].parse::<f64>().ok();
        if &c[2] == "from_value" {
            e.0 = v
        } else {
            e.1 = v
        }
    }
    // descriptor variable -> range
    let mut desc: std::collections::HashMap<String, Constraint> = std::collections::HashMap::new();
    let mut add = |d: &str, key: &str| {
        if let Some((Some(min), Some(max))) = ranges.get(key) {
            desc.insert(
                d.to_string(),
                Constraint::Range {
                    min: *min,
                    max: *max,
                },
            );
        }
    };
    for c in push.captures_iter(src) {
        add(&c[1], &c[2]);
    }
    for key in ranges.keys().cloned().collect::<Vec<_>>() {
        for field in [".floating_point_range[0]", ".integer_range[0]"] {
            if let Some(d) = key.strip_suffix(field) {
                add(d, &key);
            }
        }
    }
    decl.captures_iter(src)
        .filter_map(|c| desc.get(&c[2]).map(|r| (c[1].to_string(), r.clone())))
        .collect()
}

/// Prefixes of generate_parameter_library listeners: `ParamListener(node, "ns")`,
/// `make_shared<ns::ParamListener>(node, prefix)` with `const std::string prefix = "ns";` in
/// the same file. None when the prefix argument cannot be resolved to a literal.
fn param_listener_prefixes(src: &str) -> Vec<Option<String>> {
    static CALL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let call = CALL.get_or_init(|| {
        regex::Regex::new(r#"ParamListener\s*>?\s*\(\s*[^,()]+,\s*([^,()]+?)\s*[,)]"#)
            .expect("valid regex")
    });
    call.captures_iter(src)
        .map(|c| {
            let arg = c[1].trim();
            if let Some(lit) = arg.strip_prefix('"').and_then(|a| a.strip_suffix('"')) {
                return Some(lit.to_string());
            }
            if !arg.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
                return None;
            }
            let var = regex::Regex::new(&format!(
                r#"std::string\s+{}\s*(?:=\s*|\{{\s*|\(\s*)"([^"]*)""#,
                regex::escape(arg)
            ))
            .ok()?;
            var.captures(src).map(|v| v[1].to_string())
        })
        .collect()
}

fn declare_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r#"\b(?:declare_parameter|declare_or_get_parameter|declare_parameter_if_not_declared)\s*(?:<[^;()]*>)?\s*\(\s*(?:[A-Za-z_][\w:\->.()]*\s*,\s*)?"([^"\s]+)""#)
            .expect("valid regex")
    })
}

fn is_node_base(t: &str) -> bool {
    let t = t.trim().trim_start_matches("public").trim();
    let last = last_segment(t);
    matches!(last, "Node" | "LifecycleNode")
        || t.contains("rclcpp::Node")
        || t.contains("rclcpp_lifecycle::LifecycleNode")
        || t.contains("nav2_util::LifecycleNode")
        || t.contains("nav2::LifecycleNode")
}

fn executor_kind(t: &str) -> Option<ExecutorKind> {
    if t.contains("MultiThreadedExecutor") {
        Some(ExecutorKind::MultiThreaded)
    } else if t.contains("SingleThreadedExecutor") || t.contains("EventsExecutor") {
        // The EventsExecutor also runs all callbacks on one thread.
        Some(ExecutorKind::SingleThreaded)
    } else {
        None
    }
}

/// (method name, class from `Class::method` qualifier)
fn function_name(n: &Node, src: &str) -> (String, Option<String>) {
    let mut d = n.child_by_field_name("declarator");
    while let Some(x) = d {
        match x.kind() {
            "function_declarator" => {
                let inner = x
                    .child_by_field_name("declarator")
                    .map(|i| text(&i, src))
                    .unwrap_or("");
                let inner = inner.trim();
                if let Some(i) = inner.rfind("::") {
                    let class = last_segment(&inner[..i]).to_string();
                    return (inner[i + 2..].to_string(), Some(class));
                }
                return (inner.to_string(), None);
            }
            _ => d = x.child_by_field_name("declarator"),
        }
    }
    (String::new(), None)
}

pub fn extract(file: FileId, src: &str) -> Result<CodeFacts, String> {
    let mut parser = Parser::new();
    parser
        .set_language(tree_sitter_cpp::language())
        .map_err(|e| e.to_string())?;
    let tree = parser
        .parse(src, None)
        .ok_or("tree-sitter returned no tree")?;
    let mut w = W {
        file,
        src,
        facts: CodeFacts::default(),
        impls: HashMap::new(),
        free_impl: None,
        file_open: false,
        future_vars: HashMap::new(),
        opt_groups: HashMap::new(),
        lambda_vars: HashMap::new(),
        qos_events: Vec::new(),
    };
    w.walk(tree.root_node(), None, None);
    let mut facts = w.facts;
    // Plugin loading only opens dotted (per-plugin) names; the resolver handles that.
    if w.file_open {
        for n in &mut facts.nodes {
            n.params_open = true;
        }
    }
    // Text fallback: declarations tree-sitter misparses (e.g. `node->declare_or_get_parameter<T>("x")`
    // reads as a `<` comparison without type information). Names found here are only used to
    // *accept* parameters, never to report them.
    let known: std::collections::HashSet<String> = facts
        .nodes
        .iter()
        .flat_map(|n| n.params.iter().map(|p| p.name.literal().to_string()))
        .chain(
            facts
                .class_params
                .iter()
                .filter_map(|(_, d)| d.as_ref().map(|d| d.name.literal().to_string())),
        )
        .collect();
    facts.param_prefixes = param_listener_prefixes(src);
    for (name, range) in descriptor_ranges(src)
        .into_iter()
        .chain(super::guards::guard_constraints(src))
    {
        for d in facts.nodes.iter_mut().flat_map(|n| n.params.iter_mut()) {
            if d.name == NamePattern::Exact(name.clone()) && !d.constraints.contains(&range) {
                d.constraints.push(range.clone());
            }
        }
    }
    for cap in declare_re().captures_iter(src) {
        let name = cap[1].to_string();
        if !known.contains(&name) {
            let m = cap.get(0).unwrap();
            let loc = Loc::new(file, m.start(), m.end());
            facts.class_params.push((
                None,
                Some(ParamDecl {
                    name: NamePattern::Exact(name),
                    ty: ParamType::Unknown,
                    has_default: true,
                    default_text: None,
                    loc,
                    constraints: Vec::new(),
                }),
            ));
        }
    }
    // Fill per-function call lists from call sites.
    for f in &mut facts.functions {
        f.calls = facts
            .call_sites
            .iter()
            .filter(|(_, p)| *p >= f.start && *p < f.end)
            .map(|(c, _)| c.clone())
            .collect();
    }
    // Drop impls that have nothing ROS-related (e.g. a free impl created only for a callback group).
    facts.nodes.retain(|n| {
        n.default_name.is_some()
            || !n.params.is_empty()
            || !n.endpoints.is_empty()
            || !n.callbacks.is_empty()
            || n.class_name.is_some()
    });
    Ok(facts)
}

#[cfg(test)]
mod duration_tests {
    use super::duration_ms;

    #[test]
    fn literal_durations() {
        for (t, ms) in [
            ("100ms", 100.0),
            ("1s", 1000.0),
            ("0.5s", 500.0),
            ("std::chrono::milliseconds(20)", 20.0),
            ("std::chrono::seconds{2}", 2000.0),
            ("std::chrono::duration<double>(0.1)", 100.0),
            ("std::chrono::duration<double, std::milli>(5)", 5.0),
            ("rclcpp::Duration::from_seconds(0.05)", 50.0),
            ("1s / 30", 1000.0 / 30.0),
            ("500us", 0.5),
        ] {
            assert_eq!(duration_ms(t), Some(ms), "{t}");
        }
        for t in [
            "period_",
            "std::chrono::milliseconds(period_ms)",
            "1s / rate",
            "std::chrono::milliseconds(1000 / 30)",
        ] {
            assert_eq!(duration_ms(t), None, "{t}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODE: &str = r#"
#include <rclcpp/rclcpp.hpp>
#include <std_msgs/msg/string.hpp>
class Talker : public rclcpp::Node {
public:
  Talker() : Node("talker") {
    declare_parameter("rate", 10.0);
    declare_parameter<std::string>("frame");
    this->declare_parameter("names", std::vector<std::string>{});
    group_ = create_callback_group(rclcpp::CallbackGroupType::MutuallyExclusive);
    client_ = create_client<std_srvs::srv::Trigger>("trigger");
    rclcpp::QoS qos(5);
    qos.best_effort();
    pub_ = create_publisher<std_msgs::msg::String>("chatter", qos);
    sub_ = create_subscription<std_msgs::msg::String>("in", rclcpp::SensorDataQoS(),
      std::bind(&Talker::on_msg, this, std::placeholders::_1));
    timer_ = create_wall_timer(std::chrono::seconds(1), [this]() { tick(); });
  }
  void on_msg(const std_msgs::msg::String::SharedPtr) {
    auto fut = client_->async_send_request(std::make_shared<std_srvs::srv::Trigger::Request>());
    fut.wait();
    auto p = std::make_shared<int>(1);
    p.get();
  }
  void tick() { std::this_thread::sleep_for(std::chrono::milliseconds(10)); }
};
int main(int argc, char ** argv) {
  rclcpp::init(argc, argv);
  rclcpp::spin(std::make_shared<Talker>());
}
"#;

    #[test]
    fn extracts_node_facts() {
        let f = extract(0, NODE).unwrap();
        let n = f
            .nodes
            .iter()
            .find(|n| n.class_name.as_deref() == Some("Talker"))
            .unwrap();
        assert_eq!(n.default_name.as_deref(), Some("talker"));
        let p = |name: &str| {
            n.params
                .iter()
                .find(|p| p.name == NamePattern::Exact(name.into()))
                .unwrap()
        };
        assert_eq!(p("rate").ty, ParamType::Double);
        assert!(p("rate").has_default);
        assert_eq!(p("frame").ty, ParamType::Str);
        assert!(!p("frame").has_default);
        assert_eq!(p("names").ty, ParamType::StrArray);
        let pubq = &n
            .endpoints
            .iter()
            .find(|e| e.kind == EndpointKind::Publisher)
            .unwrap()
            .qos;
        assert_eq!(pubq.reliability, Some(Reliability::BestEffort));
        assert_eq!(pubq.depth, Some(5));
        let sub = n
            .endpoints
            .iter()
            .find(|e| e.kind == EndpointKind::Subscription)
            .unwrap();
        assert_eq!(sub.qos.reliability, Some(Reliability::BestEffort));
        assert_eq!(
            sub.callback,
            Some(CallbackTarget::Function("on_msg".into()))
        );
        assert_eq!(n.callbacks.len(), 2);
        // fut.wait() is a future wait; p.get() (shared_ptr) is not.
        let waits: Vec<_> = f
            .blocking
            .iter()
            .filter(|b| matches!(b.kind, BlockingKind::FutureWait { .. }))
            .collect();
        assert_eq!(waits.len(), 1);
        assert!(
            matches!(&waits[0].kind, BlockingKind::FutureWait { client: Some(c) } if c == "client_")
        );
        assert!(f.blocking.iter().any(|b| b.kind == BlockingKind::Sleep));
        assert!(f
            .executors
            .iter()
            .any(|e| e.kind == ExecutorKind::SingleThreaded));
        assert!(f.imports.iter().any(|(p, _)| p == "std_msgs"));
        assert!(!n.params_open);
    }

    #[test]
    fn descriptor_range() {
        let src = "#include <rclcpp/rclcpp.hpp>
class N : public rclcpp::Node { public: N() : Node(\"n\") {
  rcl_interfaces::msg::ParameterDescriptor d;
  rcl_interfaces::msg::FloatingPointRange r;
  r.from_value = 0.0;
  r.to_value = 2.5;
  d.floating_point_range.push_back(r);
  declare_parameter(\"max_speed\", 1.0, d);
  rcl_interfaces::msg::ParameterDescriptor e;
  e.integer_range.resize(1);
  e.integer_range[0].from_value = 1;
  e.integer_range[0].to_value = 10;
  declare_parameter<int>(\"rate\", 5, e);
  declare_parameter(\"free\", 1.0);
} };
";
        let f = extract(0, src).unwrap();
        let get = |n: &str| {
            f.nodes[0]
                .params
                .iter()
                .find(|p| p.name.literal() == n)
                .map(|p| p.constraints.clone())
                .unwrap()
        };
        assert_eq!(
            get("max_speed"),
            vec![Constraint::Range { min: 0.0, max: 2.5 }]
        );
        assert_eq!(
            get("rate"),
            vec![Constraint::Range {
                min: 1.0,
                max: 10.0
            }]
        );
        assert!(get("free").is_empty());
    }

    /// Nav2's AMCL declares every parameter through `nav2_util::LifecycleNode::add_parameter`
    /// (name, default, [range,] description, ...), a wrapper over `declare_parameter`.
    #[test]
    fn nav2_add_parameter_declares() {
        let src = r#"
AmclNode::AmclNode(const rclcpp::NodeOptions & options)
: nav2_util::LifecycleNode("amcl", "", options)
{
  add_parameter(
    "alpha1", rclcpp::ParameterValue(0.2),
    "This is the alpha1 parameter", "These are additional constraints for alpha1");
  add_parameter("base_frame_id", rclcpp::ParameterValue(std::string("base_footprint")), "frame");
  add_parameter("max_particles", rclcpp::ParameterValue(2000), "count");
}
"#;
        let f = extract(0, src).unwrap();
        let n = &f.nodes[0];
        let ty = |name: &str| {
            n.params
                .iter()
                .find(|p| p.name.literal() == name)
                .map(|p| p.ty)
        };
        assert_eq!(ty("alpha1"), Some(ParamType::Double), "{:#?}", n.params);
        assert_eq!(ty("base_frame_id"), Some(ParamType::Str));
        assert_eq!(ty("max_particles"), Some(ParamType::Int));
        assert!(!n.params_open);
    }

    #[test]
    fn plugin_prefix_params_and_foreign_decls() {
        let src = r#"
void Plugin::configure(const rclcpp_lifecycle::LifecycleNode::WeakPtr & parent, std::string name) {
  auto node = parent.lock();
  nav2_util::declare_parameter_if_not_declared(node, name + ".max_vel", rclcpp::ParameterValue(0.5));
  node->declare_parameter(name + ".k", 1.0);
}
"#;
        let f = extract(0, src).unwrap();
        assert_eq!(f.class_params.len(), 2);
        assert!(f
            .class_params
            .iter()
            .all(|(c, d)| c.as_deref() == Some("Plugin") && d.is_some()));
    }

    #[test]
    fn qos_text() {
        let q =
            qos_from_text("rclcpp::QoS(rclcpp::KeepLast(1)).transient_local().reliable()").unwrap();
        assert_eq!(q.durability, Some(Durability::TransientLocal));
        assert_eq!(q.reliability, Some(Reliability::Reliable));
        assert!(qos_from_text("qos_").is_none());
        assert_eq!(
            qos_from_text("10").unwrap().reliability,
            Some(Reliability::Reliable)
        );
    }
}
