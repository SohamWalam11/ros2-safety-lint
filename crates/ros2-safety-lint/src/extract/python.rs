//! rclpy node facts via the RustPython AST.

use crate::facts::*;
use crate::source::{FileId, Loc};
use rustpython_parser::ast::{self, Constant, Expr, Ranged, Stmt};
use std::collections::HashMap;

pub fn is_relevant(text: &str) -> bool {
    text.contains("rclpy")
}

struct W {
    file: FileId,
    facts: CodeFacts,
    impls: HashMap<String, usize>,
    free_impl: Option<usize>,
    /// future variable → client variable
    futures: HashMap<String, Option<String>>,
    /// local QoS variables (module/function scope, last assignment wins)
    qos_vars: HashMap<String, QosSpec>,
}

fn rng(e: &impl Ranged) -> (usize, usize) {
    (e.range().start().to_usize(), e.range().end().to_usize())
}

/// A Python literal default (`1.0`, `'map'`, `True`, `[1, 2]`) as text; None when computed.
fn py_literal(e: &Expr) -> Option<String> {
    match e {
        Expr::Constant(c) => match &c.value {
            Constant::Bool(b) => Some(if *b { "true" } else { "false" }.to_string()),
            Constant::Int(i) => Some(i.to_string()),
            Constant::Float(f) => Some(format!("{f:?}")),
            Constant::Str(s) => Some(format!("{s:?}")),
            _ => None,
        },
        Expr::UnaryOp(u) if matches!(u.op, ast::UnaryOp::USub) => {
            py_literal(&u.operand).map(|v| format!("-{v}"))
        }
        Expr::List(ast::ExprList { elts, .. }) | Expr::Tuple(ast::ExprTuple { elts, .. }) => {
            let items: Option<Vec<String>> = elts.iter().map(py_literal).collect();
            items.map(|v| format!("[{}]", v.join(", ")))
        }
        _ => None,
    }
}

fn expr_text(e: &Expr) -> String {
    match e {
        Expr::Name(n) => n.id.to_string(),
        Expr::Attribute(a) => format!("{}.{}", expr_text(&a.value), a.attr),
        Expr::Call(c) => format!("{}()", expr_text(&c.func)),
        Expr::Constant(c) => match &c.value {
            Constant::Str(s) => format!("'{s}'"),
            Constant::Int(i) => i.to_string(),
            Constant::Float(f) => f.to_string(),
            Constant::Bool(b) => b.to_string(),
            _ => "?".into(),
        },
        _ => "?".into(),
    }
}

/// `self.x` → `x`, `x` → `x`
fn var_name(e: &Expr) -> Option<String> {
    match e {
        Expr::Name(n) => Some(n.id.to_string()),
        Expr::Attribute(a) => match &*a.value {
            Expr::Name(n) if n.id.as_str() == "self" => Some(a.attr.to_string()),
            _ => Some(expr_text(e)),
        },
        _ => None,
    }
}

fn kw<'a>(c: &'a ast::ExprCall, name: &str) -> Option<&'a Expr> {
    c.keywords
        .iter()
        .find(|k| k.arg.as_ref().map(|a| a.as_str()) == Some(name))
        .map(|k| &k.value)
}

fn str_const(e: &Expr) -> Option<String> {
    match e {
        Expr::Constant(c) => match &c.value {
            Constant::Str(s) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// A number written as literals and + - * / (e.g. `0.1`, `1.0 / 30`).
fn py_number(e: &Expr) -> Option<f64> {
    match e {
        Expr::Constant(c) => match &c.value {
            Constant::Int(i) => i.to_string().parse().ok(),
            Constant::Float(f) => Some(*f),
            _ => None,
        },
        Expr::UnaryOp(u) if matches!(u.op, ast::UnaryOp::USub) => py_number(&u.operand).map(|x| -x),
        Expr::BinOp(b) => {
            let (l, r) = (py_number(&b.left)?, py_number(&b.right)?);
            match b.op {
                ast::Operator::Add => Some(l + r),
                ast::Operator::Sub => Some(l - r),
                ast::Operator::Mult => Some(l * r),
                ast::Operator::Div if r != 0.0 => Some(l / r),
                _ => None,
            }
        }
        _ => None,
    }
}

fn const_type(e: &Expr) -> (ParamType, bool) {
    match e {
        Expr::Constant(c) => match &c.value {
            Constant::Bool(_) => (ParamType::Bool, true),
            Constant::Int(_) => (ParamType::Int, true),
            Constant::Float(_) => (ParamType::Double, true),
            Constant::Str(_) => (ParamType::Str, true),
            Constant::None => (ParamType::Unknown, true),
            _ => (ParamType::Unknown, true),
        },
        Expr::UnaryOp(u) => const_type(&u.operand),
        Expr::List(l) => {
            let tys: Vec<ParamType> = l.elts.iter().map(|x| const_type(x).0).collect();
            let ty = if tys.is_empty() {
                ParamType::Unknown
            } else if tys.iter().all(|t| *t == ParamType::Double) {
                ParamType::DoubleArray
            } else if tys.iter().all(|t| *t == ParamType::Int) {
                ParamType::IntArray
            } else if tys.iter().all(|t| *t == ParamType::Str) {
                ParamType::StrArray
            } else if tys.iter().all(|t| *t == ParamType::Bool) {
                ParamType::BoolArray
            } else {
                ParamType::Unknown
            };
            (ty, true)
        }
        Expr::Attribute(a) => {
            // Parameter.Type.DOUBLE → typed, no default
            let ty = match a.attr.as_str() {
                "DOUBLE" => ParamType::Double,
                "INTEGER" => ParamType::Int,
                "STRING" => ParamType::Str,
                "BOOL" => ParamType::Bool,
                "DOUBLE_ARRAY" => ParamType::DoubleArray,
                "INTEGER_ARRAY" => ParamType::IntArray,
                "STRING_ARRAY" => ParamType::StrArray,
                "BOOL_ARRAY" => ParamType::BoolArray,
                _ => return (ParamType::Unknown, true),
            };
            (ty, !expr_text(e).contains("Type."))
        }
        _ => (ParamType::Unknown, true),
    }
}

impl W {
    fn loc(&self, e: &impl Ranged) -> Loc {
        let (s, t) = rng(e);
        Loc::new(self.file, s, t)
    }

    fn impl_for(&mut self, class: Option<&str>, loc: Loc) -> usize {
        match class {
            Some(c) => {
                if let Some(i) = self.impls.get(c) {
                    return *i;
                }
                self.facts
                    .nodes
                    .push(new_impl(self.file, Some(c.to_string()), loc));
                let i = self.facts.nodes.len() - 1;
                self.impls.insert(c.to_string(), i);
                i
            }
            None => {
                if let Some(i) = self.free_impl {
                    return i;
                }
                self.facts.nodes.push(new_impl(self.file, None, loc));
                let i = self.facts.nodes.len() - 1;
                self.free_impl = Some(i);
                i
            }
        }
    }

    fn qos(&self, e: Option<&Expr>) -> QosSpec {
        let Some(e) = e else {
            return QosSpec::unknown("");
        };
        let t = expr_text(e);
        match e {
            Expr::Constant(c) if matches!(c.value, Constant::Int(_)) => {
                let d = t.parse().ok();
                QosSpec {
                    depth: d,
                    ..QosSpec::ros_default(t)
                }
            }
            Expr::Name(_) | Expr::Attribute(_) => {
                let v = var_name(e).unwrap_or_default();
                if let Some(q) = self.qos_vars.get(&v) {
                    return q.clone();
                }
                let last = t.rsplit('.').next().unwrap_or(&t);
                match last {
                    "qos_profile_sensor_data" => QosSpec::sensor_data(t),
                    "qos_profile_system_default" => QosSpec::system_default(t),
                    "qos_profile_services_default"
                    | "qos_profile_parameters"
                    | "qos_profile_parameter_events" => QosSpec::ros_default(t),
                    _ => QosSpec::unknown(t),
                }
            }
            Expr::Call(c) => self.qos_call(c).unwrap_or_else(|| QosSpec::unknown(t)),
            _ => QosSpec::unknown(t),
        }
    }

    fn qos_call(&self, c: &ast::ExprCall) -> Option<QosSpec> {
        let fname = expr_text(&c.func);
        if !fname.ends_with("QoSProfile") {
            return None;
        }
        let mut q = QosSpec::ros_default("QoSProfile(...)");
        let mut parts = vec!["QoSProfile(".to_string()];
        for k in &c.keywords {
            let Some(a) = &k.arg else { continue };
            let v = expr_text(&k.value);
            parts.push(format!("{a}={v}"));
            match a.as_str() {
                "depth" => q.depth = v.parse().ok(),
                "reliability" => {
                    q.reliability = if v.contains("BEST_EFFORT") {
                        Some(Reliability::BestEffort)
                    } else if v.contains("RELIABLE") {
                        Some(Reliability::Reliable)
                    } else {
                        None
                    }
                }
                "durability" => {
                    q.durability = if v.contains("TRANSIENT_LOCAL") {
                        Some(Durability::TransientLocal)
                    } else if v.contains("VOLATILE") {
                        Some(Durability::Volatile)
                    } else {
                        None
                    }
                }
                "deadline" => q.deadline_ns = None,
                "liveliness" => q.liveliness = None,
                "liveliness_lease_duration" => q.lease_ns = None,
                _ => {}
            }
        }
        q.origin = format!("{})", parts.join(", ").replacen("(, ", "(", 1));
        Some(q)
    }

    fn callback_target(&self, e: &Expr) -> Option<CallbackTarget> {
        match e {
            Expr::Lambda(l) => {
                let (s, t) = rng(&*l.body);
                Some(CallbackTarget::Inline { start: s, end: t })
            }
            Expr::Attribute(a) if matches!(&*a.value, Expr::Name(n) if n.id.as_str() == "self") => {
                Some(CallbackTarget::Function(a.attr.to_string()))
            }
            Expr::Name(n) => Some(CallbackTarget::Function(n.id.to_string())),
            _ => None,
        }
    }

    fn stmts(&mut self, body: &[Stmt], class: Option<&str>, in_node_class: bool) {
        for s in body {
            self.stmt(s, class, in_node_class);
        }
    }

    fn stmt(&mut self, s: &Stmt, class: Option<&str>, in_node_class: bool) {
        match s {
            Stmt::ClassDef(c) => {
                let is_node = c.bases.iter().any(|b| {
                    let t = expr_text(b);
                    t == "Node" || t.ends_with(".Node") || t.ends_with("LifecycleNode")
                });
                let name = c.name.to_string();
                if !self.facts.classes_defined.contains(&name) {
                    self.facts.classes_defined.push(name.clone());
                }
                if is_node {
                    let loc = self.loc(c);
                    let i = self.impl_for(Some(&name), loc);
                    self.facts.nodes[i].base = c.bases.first().map(expr_text);
                }
                self.stmts(&c.body, Some(&name), is_node);
            }
            Stmt::FunctionDef(f) => {
                let (s, e) = rng(f);
                self.facts.functions.push(FunctionInfo {
                    name: f.name.to_string(),
                    class: class.map(|c| c.to_string()),
                    start: s,
                    end: e,
                    calls: Vec::new(),
                });
                self.stmts(&f.body, class, in_node_class);
            }
            Stmt::AsyncFunctionDef(f) => {
                let (s, e) = rng(f);
                self.facts.functions.push(FunctionInfo {
                    name: f.name.to_string(),
                    class: class.map(|c| c.to_string()),
                    start: s,
                    end: e,
                    calls: Vec::new(),
                });
                self.stmts(&f.body, class, in_node_class);
            }
            Stmt::Assign(a) => {
                let target = a.targets.first().and_then(var_name);
                self.expr(&a.value, class, in_node_class, target.as_deref());
            }
            Stmt::AnnAssign(a) => {
                if let Some(v) = &a.value {
                    let target = var_name(&a.target);
                    self.expr(v, class, in_node_class, target.as_deref());
                }
            }
            Stmt::Expr(e) => self.expr(&e.value, class, in_node_class, None),
            Stmt::Return(r) => {
                if let Some(v) = &r.value {
                    self.expr(v, class, in_node_class, None);
                }
            }
            Stmt::If(i) => {
                self.expr(&i.test, class, in_node_class, None);
                self.stmts(&i.body, class, in_node_class);
                self.stmts(&i.orelse, class, in_node_class);
            }
            Stmt::While(w) => {
                self.expr(&w.test, class, in_node_class, None);
                self.stmts(&w.body, class, in_node_class);
            }
            Stmt::For(f) => self.stmts(&f.body, class, in_node_class),
            Stmt::AsyncFor(f) => self.stmts(&f.body, class, in_node_class),
            Stmt::With(w) => self.stmts(&w.body, class, in_node_class),
            Stmt::AsyncWith(w) => self.stmts(&w.body, class, in_node_class),
            Stmt::Try(t) => {
                self.stmts(&t.body, class, in_node_class);
                for h in &t.handlers {
                    let ast::ExceptHandler::ExceptHandler(h) = h;
                    self.stmts(&h.body, class, in_node_class);
                }
                self.stmts(&t.orelse, class, in_node_class);
                self.stmts(&t.finalbody, class, in_node_class);
            }
            Stmt::Import(i) => {
                for a in &i.names {
                    let top = a.name.split('.').next().unwrap_or("").to_string();
                    let loc = self.loc(a);
                    self.facts.imports.push((top, loc));
                }
            }
            Stmt::ImportFrom(i) if i.level.is_none_or(|l| l.to_u32() == 0) => {
                if let Some(m) = &i.module {
                    let top = m.split('.').next().unwrap_or("").to_string();
                    let loc = self.loc(i);
                    self.facts.imports.push((top, loc));
                }
            }
            _ => {}
        }
    }

    fn expr(&mut self, e: &Expr, class: Option<&str>, in_node_class: bool, target: Option<&str>) {
        match e {
            Expr::Call(c) => self.call(c, class, in_node_class, target),
            Expr::Await(a) => self.expr(&a.value, class, in_node_class, target),
            Expr::BoolOp(b) => {
                for v in &b.values {
                    self.expr(v, class, in_node_class, None);
                }
            }
            Expr::UnaryOp(u) => self.expr(&u.operand, class, in_node_class, None),
            Expr::Compare(c) => {
                self.expr(&c.left, class, in_node_class, None);
                for x in &c.comparators {
                    self.expr(x, class, in_node_class, None);
                }
            }
            Expr::Attribute(a) => self.expr(&a.value, class, in_node_class, None),
            _ => {}
        }
    }

    fn call(
        &mut self,
        c: &ast::ExprCall,
        class: Option<&str>,
        in_node_class: bool,
        target: Option<&str>,
    ) {
        // Evaluate nested calls in receivers/args first (source order).
        if let Expr::Attribute(a) = &*c.func {
            self.expr(&a.value, class, in_node_class, None);
        }
        for a in &c.args {
            self.expr(a, class, in_node_class, None);
        }
        let (fname, recv) = match &*c.func {
            Expr::Name(n) => (n.id.to_string(), None),
            Expr::Attribute(a) => (a.attr.to_string(), Some(&*a.value)),
            _ => return,
        };
        let (start, _) = rng(c);
        self.facts.call_sites.push((fname.clone(), start));
        let recv_is_self =
            recv.is_some_and(|r| matches!(r, Expr::Name(n) if n.id.as_str() == "self"));
        let recv_var = recv.and_then(var_name);
        let loc = self.loc(c);
        // Which node impl does this belong to?
        let node_target = |w: &mut W| -> Option<usize> {
            if in_node_class && recv_is_self {
                Some(w.impl_for(class, loc))
            } else if class.is_none() || !in_node_class {
                if recv_is_self {
                    None
                } else {
                    Some(w.impl_for(None, loc))
                }
            } else {
                // node class, but via another handle (e.g. self.node.create_publisher)
                None
            }
        };

        match fname.as_str() {
            "__init__" if in_node_class => {
                // super().__init__('name', ...)
                if let Some(n) = c
                    .args
                    .first()
                    .and_then(str_const)
                    .or_else(|| kw(c, "node_name").and_then(str_const))
                {
                    let i = self.impl_for(class, loc);
                    self.facts.nodes[i].default_name = Some(n);
                    self.facts.nodes[i].loc = loc;
                }
                if kw(c, "allow_undeclared_parameters").is_some()
                    || kw(c, "automatically_declare_parameters_from_overrides").is_some()
                {
                    let i = self.impl_for(class, loc);
                    self.facts.nodes[i].params_open = true;
                }
            }
            "create_node" | "Node" if !in_node_class => {
                if let Some(n) = c.args.first().and_then(str_const) {
                    let i = self.impl_for(None, loc);
                    self.facts.nodes[i].default_name = Some(n);
                    self.facts.nodes[i].loc = loc;
                }
            }
            "declare_parameter" => {
                let Some(i) = node_target(self) else {
                    let decl = c.args.first().and_then(str_const).map(|name| {
                        let (ty, has_default) = c
                            .args
                            .get(1)
                            .or(kw(c, "value"))
                            .map_or((ParamType::Unknown, true), const_type);
                        ParamDecl {
                            name: NamePattern::Exact(name),
                            ty,
                            has_default,
                            default_text: c.args.get(1).or(kw(c, "value")).and_then(py_literal),
                            loc,
                            constraints: Vec::new(),
                        }
                    });
                    self.facts
                        .class_params
                        .push((class.map(|c| c.to_string()), decl));
                    return;
                };
                match c.args.first().and_then(str_const) {
                    Some(name) => {
                        let (ty, has_default) = c
                            .args
                            .get(1)
                            .or(kw(c, "value"))
                            .map_or((ParamType::Unknown, true), const_type);
                        self.facts.nodes[i].params.push(ParamDecl {
                            name: NamePattern::Exact(name),
                            ty,
                            has_default,
                            default_text: c.args.get(1).or(kw(c, "value")).and_then(py_literal),
                            loc,
                            constraints: Vec::new(),
                        });
                    }
                    None => self.facts.nodes[i].params_open = true,
                }
            }
            "declare_parameters" => {
                let Some(i) = node_target(self) else {
                    self.facts
                        .class_params
                        .push((class.map(|c| c.to_string()), None));
                    return;
                };
                let ns = c
                    .args
                    .first()
                    .or(kw(c, "namespace"))
                    .and_then(str_const)
                    .unwrap_or_default();
                match c.args.get(1).or(kw(c, "parameters")) {
                    Some(Expr::List(l)) => {
                        for el in &l.elts {
                            let (name, val) = match el {
                                Expr::Tuple(t) => {
                                    (t.elts.first().and_then(str_const), t.elts.get(1))
                                }
                                other => (str_const(other), None),
                            };
                            match name {
                                Some(n) => {
                                    let full = if ns.is_empty() {
                                        n
                                    } else {
                                        format!("{ns}.{n}")
                                    };
                                    let (ty, has_default) =
                                        val.map_or((ParamType::Unknown, true), const_type);
                                    self.facts.nodes[i].params.push(ParamDecl {
                                        name: NamePattern::Exact(full),
                                        ty,
                                        has_default,
                                        default_text: val.and_then(py_literal),
                                        loc,
                                        constraints: Vec::new(),
                                    });
                                }
                                None => self.facts.nodes[i].params_open = true,
                            }
                        }
                    }
                    _ => self.facts.nodes[i].params_open = true,
                }
            }
            "create_publisher"
            | "create_subscription"
            | "create_client"
            | "create_service"
            | "create_timer"
            | "ActionClient"
            | "ActionServer" => {
                let is_action = fname.starts_with("Action");
                // ActionClient(node, Type, name, ...): node is arg 0
                let ep_target = if is_action {
                    match c.args.first() {
                        Some(Expr::Name(n)) if n.id.as_str() == "self" && in_node_class => {
                            Some(self.impl_for(class, loc))
                        }
                        _ if !in_node_class => Some(self.impl_for(None, loc)),
                        _ => None,
                    }
                } else {
                    node_target(self)
                };
                let Some(i) = ep_target else { return };
                let a: Vec<&Expr> = c.args.iter().skip(usize::from(is_action)).collect();
                let group = kw(c, "callback_group").and_then(var_name);
                let overrides = kw(c, "qos_overriding_options").is_some();
                let msg_type = a.first().map(|t| expr_text(t));
                let name = a
                    .get(1)
                    .and_then(|x| str_const(x))
                    .or_else(|| kw(c, "topic").and_then(str_const));
                let pub_qos = self.qos(a.get(2).copied().or(kw(c, "qos_profile")));
                match fname.as_str() {
                    "create_publisher" => self.facts.nodes[i].endpoints.push(Endpoint {
                        kind: EndpointKind::Publisher,
                        name,
                        msg_type,
                        qos: pub_qos,
                        qos_overrides_enabled: overrides,
                        var: target.map(|t| t.to_string()),
                        group,
                        callback: None,
                        loc,
                    }),
                    "create_subscription" => {
                        let cb = a
                            .get(2)
                            .copied()
                            .or(kw(c, "callback"))
                            .and_then(|x| self.callback_target(x));
                        if let Some(cb) = &cb {
                            self.facts.nodes[i].callbacks.push(Callback {
                                kind: CallbackKind::Subscription,
                                target: cb.clone(),
                                group: group.clone(),
                                period_ms: None,
                                loc,
                            });
                        }
                        let qos = self.qos(a.get(3).copied().or(kw(c, "qos_profile")));
                        self.facts.nodes[i].endpoints.push(Endpoint {
                            kind: EndpointKind::Subscription,
                            name,
                            msg_type,
                            qos,
                            qos_overrides_enabled: overrides,
                            var: target.map(|t| t.to_string()),
                            group,
                            callback: cb,
                            loc,
                        });
                    }
                    "create_client" | "ActionClient" => {
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
                            var: target.map(|t| t.to_string()),
                            group,
                            callback: None,
                            loc,
                        })
                    }
                    "create_service" | "ActionServer" => {
                        let cb_expr = if is_action {
                            kw(c, "execute_callback").or(a.get(2).copied())
                        } else {
                            a.get(2).copied().or(kw(c, "callback"))
                        };
                        let cb = cb_expr.and_then(|x| self.callback_target(x));
                        if let Some(cb) = &cb {
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
                            var: target.map(|t| t.to_string()),
                            group,
                            callback: cb,
                            loc,
                        });
                    }
                    _ => {
                        // create_timer(period, callback, callback_group=...)
                        let cb = c
                            .args
                            .get(1)
                            .or(kw(c, "callback"))
                            .and_then(|x| self.callback_target(x));
                        if let Some(cb) = cb {
                            self.facts.nodes[i].callbacks.push(Callback {
                                kind: CallbackKind::Timer,
                                target: cb,
                                group,
                                period_ms: c
                                    .args
                                    .first()
                                    .or(kw(c, "timer_period_sec"))
                                    .and_then(py_number)
                                    .map(|s| s * 1000.0),
                                loc,
                            });
                        }
                    }
                }
            }
            "MutuallyExclusiveCallbackGroup" | "ReentrantCallbackGroup" => {
                if let Some(t) = target {
                    let kind = if fname.starts_with("Reentrant") {
                        GroupKind::Reentrant
                    } else {
                        GroupKind::MutuallyExclusive
                    };
                    let i = if in_node_class {
                        self.impl_for(class, loc)
                    } else {
                        self.impl_for(None, loc)
                    };
                    self.facts.nodes[i].groups.push(CallbackGroupDecl {
                        var: t.to_string(),
                        kind,
                        not_auto_added: false,
                        loc,
                    });
                }
            }
            "MultiThreadedExecutor" | "SingleThreadedExecutor" => {
                let kind = if fname.starts_with("Multi") {
                    ExecutorKind::MultiThreaded
                } else {
                    ExecutorKind::SingleThreaded
                };
                self.facts.executors.push(ExecutorUse {
                    kind,
                    var: target.map(|t| t.to_string()),
                    loc,
                });
            }
            "add_callback_group" => {
                if let Some(g) = c.args.first().and_then(var_name) {
                    for n in &mut self.facts.nodes {
                        n.dedicated_groups.push(g.clone());
                    }
                }
            }
            "spin" if recv_var.as_deref() == Some("rclpy") => {
                self.facts.executors.push(ExecutorUse {
                    kind: ExecutorKind::SingleThreaded,
                    var: None,
                    loc,
                });
            }
            "spin_until_future_complete" | "spin_once" => {
                let executor = match recv_var.as_deref() {
                    Some("rclpy") | None => None,
                    Some(v) => Some(v.to_string()),
                };
                self.facts.blocking.push(BlockingCall {
                    kind: BlockingKind::Spin { executor },
                    loc,
                });
            }
            "call_async" | "send_goal_async" | "get_result_async" => {
                if let Some(t) = target {
                    self.futures.insert(t.to_string(), recv_var.clone());
                }
            }
            "call" | "send_goal" => {
                // Synchronous service/action call.
                if recv_var.is_some() && !recv_is_self {
                    self.facts.blocking.push(BlockingCall {
                        kind: BlockingKind::SyncCall {
                            client: recv_var.clone(),
                        },
                        loc,
                    });
                }
            }
            "result"
                if recv_var
                    .as_deref()
                    .is_some_and(|r| self.futures.contains_key(r)) =>
            {
                // future.result() does not block in rclpy; ignore.
            }
            "sleep" => {
                let r = recv_var.as_deref().unwrap_or("");
                if r == "time" || r.to_ascii_lowercase().contains("rate") {
                    self.facts.blocking.push(BlockingCall {
                        kind: BlockingKind::Sleep,
                        loc,
                    });
                }
            }
            "sleep_for" | "sleep_until" => self.facts.blocking.push(BlockingCall {
                kind: BlockingKind::Sleep,
                loc,
            }),
            "QoSProfile" => {
                if let (Some(t), Some(q)) = (target, self.qos_call(c)) {
                    self.qos_vars.insert(t.to_string(), q);
                }
            }
            _ => {}
        }
    }
}

fn new_impl(file: FileId, class: Option<String>, loc: Loc) -> NodeImpl {
    NodeImpl {
        file,
        lang: Lang::Python,
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

pub fn extract(file: FileId, rel_path: &str, src: &str) -> Result<CodeFacts, String> {
    let m = rustpython_parser::parse(src, rustpython_parser::Mode::Module, rel_path)
        .map_err(|e| e.to_string())?;
    let ast::Mod::Module(m) = m else {
        return Err("not a module".into());
    };
    let mut w = W {
        file,
        facts: CodeFacts::default(),
        impls: HashMap::new(),
        free_impl: None,
        futures: HashMap::new(),
        qos_vars: HashMap::new(),
    };
    w.stmts(&m.body, None, false);
    let mut facts = w.facts;
    for f in &mut facts.functions {
        f.calls = facts
            .call_sites
            .iter()
            .filter(|(_, p)| *p >= f.start && *p < f.end)
            .map(|(c, _)| c.clone())
            .collect();
    }
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
mod tests {
    use super::*;

    const SRC: &str = r#"
import time
import rclpy
from rclpy.node import Node
from rclpy.qos import QoSProfile, ReliabilityPolicy, DurabilityPolicy, qos_profile_sensor_data
from std_srvs.srv import Trigger
from std_msgs.msg import String

class Listener(Node):
    def __init__(self):
        super().__init__('listener')
        self.declare_parameter('gain', 1.5)
        self.declare_parameter('mode', 'fast')
        self.cli = self.create_client(Trigger, 'trigger')
        qos = QoSProfile(depth=1, durability=DurabilityPolicy.TRANSIENT_LOCAL)
        self.sub = self.create_subscription(String, 'map', self.on_msg, qos)
        self.sub2 = self.create_subscription(String, 'scan', self.on_scan, qos_profile_sensor_data)

    def on_msg(self, msg):
        res = self.cli.call(Trigger.Request())
        time.sleep(0.1)

    def on_scan(self, msg):
        pass

def main():
    rclpy.init()
    rclpy.spin(Listener())
"#;

    #[test]
    fn timer_periods() {
        let src = "from rclpy.node import Node\nclass T(Node):\n    def __init__(self):\n        super().__init__('t')\n        self.a = self.create_timer(0.1, self.tick)\n        self.b = self.create_timer(1.0 / 20, self.tick)\n        self.c = self.create_timer(self.period, self.tick)\n        self.d = self.create_timer(timer_period_sec=2, callback=self.tick)\n    def tick(self):\n        pass\n";
        let f = extract(0, "t.py", src).unwrap();
        let p: Vec<Option<f64>> = f.nodes[0].callbacks.iter().map(|c| c.period_ms).collect();
        assert_eq!(p, [Some(100.0), Some(50.0), None, Some(2000.0)]);
    }

    #[test]
    fn extracts() {
        let f = extract(0, "x.py", SRC).unwrap();
        let n = &f.nodes[0];
        assert_eq!(n.default_name.as_deref(), Some("listener"));
        assert_eq!(n.params.len(), 2);
        assert_eq!(n.params[0].ty, ParamType::Double);
        let map = n
            .endpoints
            .iter()
            .find(|e| e.name.as_deref() == Some("map"))
            .unwrap();
        assert_eq!(map.qos.durability, Some(Durability::TransientLocal));
        let scan = n
            .endpoints
            .iter()
            .find(|e| e.name.as_deref() == Some("scan"))
            .unwrap();
        assert_eq!(scan.qos.reliability, Some(Reliability::BestEffort));
        assert_eq!(n.callbacks.len(), 2);
        assert!(f
            .blocking
            .iter()
            .any(|b| matches!(&b.kind, BlockingKind::SyncCall { client: Some(c) } if c == "cli")));
        assert!(f.blocking.iter().any(|b| b.kind == BlockingKind::Sleep));
        assert!(f
            .executors
            .iter()
            .any(|e| e.kind == ExecutorKind::SingleThreaded));
        assert!(f
            .functions
            .iter()
            .any(|x| x.name == "on_msg" && x.class.as_deref() == Some("Listener")));
    }
}
