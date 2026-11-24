//! Launch files (XML, YAML and Python) → a common [`LaunchItem`] tree.
//!
//! Python launch files are *not* executed. A small static interpreter evaluates the
//! common subset (Node, IncludeLaunchDescription, GroupAction, PushRosNamespace,
//! SetParameter, DeclareLaunchArgument, LaunchConfiguration, PathJoinSubstitution,
//! FindPackageShare, os.path.join, RewrittenYaml, helper functions, list building).
//! Anything else becomes `Unknown` and marks the file `partial`.
//!
//! Workspace-relative paths (from `$(dirname)` / `__file__`) are encoded as
//! `LVal::Lit("@ws:<dir>")` and resolved by the resolver.

use super::yaml::{self, YNode};
use crate::facts::{
    LVal, LaunchFile, LaunchItem, LaunchNodeDecl, LaunchNodeKind, LaunchParamSource, ParamValue,
};
use crate::source::{FileId, Loc};
use rustpython_parser::ast::{self, Constant, Expr, Ranged, Stmt};
use std::collections::HashMap;

pub const WS_PREFIX: &str = "@ws:";

pub fn is_python_launch(rel_path: &str, text: &str) -> bool {
    let name = rel_path.rsplit('/').next().unwrap_or(rel_path);
    (name.ends_with(".launch.py") || name.ends_with("_launch.py") || rel_path.contains("/launch/") && name.ends_with(".py"))
        && text.contains("LaunchDescription")
        // launch_testing files define generate_test_description() instead.
        && text.contains("generate_launch_description")
}

pub fn is_xml_launch(rel_path: &str, text: &str) -> bool {
    let name = rel_path.rsplit('/').next().unwrap_or(rel_path);
    (name.ends_with(".launch.xml")
        || name.ends_with(".launch")
        || (rel_path.contains("/launch/") && name.ends_with(".xml")))
        && text.contains("<launch")
}

pub fn is_yaml_launch(rel_path: &str, text: &str) -> bool {
    let name = rel_path.rsplit('/').next().unwrap_or(rel_path);
    (name.ends_with(".launch.yaml")
        || name.ends_with(".launch.yml")
        || rel_path.contains("/launch/"))
        && (name.ends_with(".yaml") || name.ends_with(".yml"))
        && text.trim_start().starts_with("launch:")
}

fn dir_of(rel_path: &str) -> String {
    match rel_path.rfind('/') {
        Some(i) => rel_path[..i].to_string(),
        None => String::new(),
    }
}

// ============================================================================ substitutions

/// Parses a frontend substitution string such as `$(find-pkg-share pkg)/config/$(var f)`.
pub fn parse_subst(s: &str, file_dir: &str) -> LVal {
    let mut parts = Vec::new();
    let mut rest = s;
    while let Some(p) = rest.find("$(") {
        if p > 0 {
            parts.push(LVal::Lit(rest[..p].to_string()));
        }
        // find matching ')'
        let after = &rest[p + 2..];
        let mut depth = 1;
        let mut end = None;
        for (i, c) in after.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(e) = end else {
            parts.push(LVal::Unknown);
            rest = "";
            break;
        };
        let inner = after[..e].trim();
        let (cmd, arg) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
        let arg = arg.trim();
        let v = match cmd {
            "var" | "arg" => LVal::Arg(arg.to_string()),
            "find-pkg-share" => match parse_subst(arg, file_dir) {
                LVal::Lit(p) => LVal::PkgShare(p),
                _ => LVal::Unknown,
            },
            "find-pkg-prefix" => match parse_subst(arg, file_dir) {
                LVal::Lit(p) => LVal::PkgPrefix(p),
                _ => LVal::Unknown,
            },
            "dirname" => LVal::Lit(format!("{WS_PREFIX}{file_dir}")),
            _ => LVal::Unknown,
        };
        parts.push(v);
        rest = &after[e + 1..];
    }
    if !rest.is_empty() {
        parts.push(LVal::Lit(rest.to_string()));
    }
    simplify(LVal::Concat(parts))
}

fn simplify(v: LVal) -> LVal {
    match v {
        LVal::Concat(parts) => {
            let mut out: Vec<LVal> = Vec::new();
            for p in parts {
                let p = simplify(p);
                match (out.last_mut(), &p) {
                    (Some(LVal::Lit(a)), LVal::Lit(b)) => a.push_str(b),
                    _ => out.push(p),
                }
            }
            match out.len() {
                0 => LVal::Lit(String::new()),
                1 => out.pop().unwrap(),
                _ => LVal::Concat(out),
            }
        }
        LVal::PathJoin(parts) => LVal::PathJoin(parts.into_iter().map(simplify).collect()),
        other => other,
    }
}

// ============================================================================ XML

pub fn parse_xml(file: FileId, rel_path: &str, text: &str) -> Result<LaunchFile, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if !root.has_tag_name("launch") {
        return Err("root element is not <launch>".into());
    }
    let dir = dir_of(rel_path);
    let mut partial = false;
    let items = xml_children(file, &dir, root, &mut partial);
    Ok(LaunchFile {
        file,
        items,
        partial,
    })
}

fn xml_children(
    file: FileId,
    dir: &str,
    parent: roxmltree::Node,
    partial: &mut bool,
) -> Vec<LaunchItem> {
    let mut items = Vec::new();
    let sub = |s: &str| parse_subst(s, dir);
    for n in parent.children().filter(|n| n.is_element()) {
        let r = n.range();
        let loc = Loc::new(file, r.start, r.end);
        let conditional = n.attribute("if").is_some() || n.attribute("unless").is_some();
        match n.tag_name().name() {
            "arg" => items.push(LaunchItem::Arg {
                name: n.attribute("name").unwrap_or_default().to_string(),
                default: n.attribute("default").or(n.attribute("value")).map(sub),
                loc,
            }),
            "let" => items.push(LaunchItem::Arg {
                name: n.attribute("name").unwrap_or_default().to_string(),
                default: n.attribute("value").map(sub),
                loc,
            }),
            "node" | "lifecycle_node" | "node_container" => {
                if n.attribute("type").is_some() && n.attribute("exec").is_none() {
                    // ROS 1 launch syntax; not analysed.
                    *partial = true;
                    continue;
                }
                let kind = match n.tag_name().name() {
                    "lifecycle_node" => LaunchNodeKind::LifecycleNode,
                    "node_container" => LaunchNodeKind::Container,
                    _ => LaunchNodeKind::Node,
                };
                let mut decl = xml_node_decl(file, dir, n, kind, conditional);
                items.push(LaunchItem::Node(decl.clone()));
                if kind == LaunchNodeKind::Container {
                    for c in n.children().filter(|c| c.has_tag_name("composable_node")) {
                        let cr = c.range();
                        decl = xml_node_decl(file, dir, c, LaunchNodeKind::Composable, conditional);
                        decl.loc = Loc::new(file, cr.start, cr.end);
                        items.push(LaunchItem::Node(decl));
                    }
                }
            }
            "include" => {
                let args = n
                    .children()
                    .filter(|c| c.has_tag_name("arg"))
                    .map(|c| {
                        (
                            c.attribute("name").unwrap_or_default().to_string(),
                            sub(c.attribute("value").unwrap_or_default()),
                        )
                    })
                    .collect();
                items.push(LaunchItem::Include {
                    path: sub(n.attribute("file").unwrap_or_default()),
                    args,
                    conditional,
                    loc,
                });
            }
            "group" => {
                // `<push-ros-namespace>` children are handled as scope pushes inside the group.
                let ns = n.attribute("ns").map(sub);
                let scoped = n
                    .attribute("scoped")
                    .map_or(true, |s| s != "false" && s != "False");
                let inner = xml_children(file, dir, n, partial);
                // `<group if=…>` / `unless=…`: everything inside runs only on that branch.
                let inner = if conditional {
                    mark_conditional(inner)
                } else {
                    inner
                };
                items.push(LaunchItem::Group {
                    namespace: ns,
                    items: inner,
                    scoped,
                    loc,
                });
            }
            "push-ros-namespace" | "push_ros_namespace" => {
                // A push outside a group applies to the remainder of the enclosing scope.
                let ns = n.attribute("namespace").or(n.attribute("ns")).map(sub);
                let rest: Vec<LaunchItem> = Vec::new();
                items.push(LaunchItem::Group {
                    namespace: ns,
                    items: rest,
                    scoped: false,
                    loc,
                });
            }
            "set_parameter" | "set-parameter" => items.push(LaunchItem::SetParam {
                name: n.attribute("name").unwrap_or_default().to_string(),
                value: sub(n.attribute("value").unwrap_or_default()),
                loc,
            }),
            "set_remap" | "set-remap" => items.push(LaunchItem::SetRemap {
                from: sub(n.attribute("from").unwrap_or_default()),
                to: sub(n.attribute("to").unwrap_or_default()),
                loc,
            }),
            "set_env" | "set-env" | "env" => items.push(LaunchItem::SetEnv {
                name: n.attribute("name").unwrap_or_default().to_string(),
                value: sub(n.attribute("value").unwrap_or_default()),
                loc,
            }),
            "executable" | "load_composable_node" | "timer" | "opaque_function" => *partial = true,
            _ => {}
        }
    }
    items
}

fn xml_node_decl(
    file: FileId,
    dir: &str,
    n: roxmltree::Node,
    kind: LaunchNodeKind,
    conditional: bool,
) -> LaunchNodeDecl {
    let sub = |s: &str| parse_subst(s, dir);
    let r = n.range();
    let mut params = Vec::new();
    let mut remaps = Vec::new();
    let mut enclave = None;
    for c in n.children().filter(|c| c.is_element()) {
        let cr = c.range();
        let cloc = Loc::new(file, cr.start, cr.end);
        match c.tag_name().name() {
            "param" => {
                if let Some(from) = c.attribute("from") {
                    params.push(LaunchParamSource::File {
                        path: sub(from),
                        root_key: None,
                        loc: cloc,
                    });
                } else if let Some(name) = c.attribute("name") {
                    let raw = c.attribute("value").unwrap_or_default();
                    let typed = match sub(raw) {
                        LVal::Lit(s) => Some(yaml::scalar_value(&s, false)),
                        _ => None,
                    };
                    params.push(LaunchParamSource::Inline {
                        name: name.to_string(),
                        value: sub(raw),
                        typed,
                        loc: cloc,
                    });
                }
            }
            "remap" => remaps.push((
                sub(c.attribute("from").unwrap_or_default()),
                sub(c.attribute("to").unwrap_or_default()),
                cloc,
            )),
            "env" if c.attribute("name") == Some("ROS_SECURITY_ENCLAVE_OVERRIDE") => {
                enclave = c.attribute("value").map(sub);
            }
            _ => {}
        }
    }
    let mut arguments = Vec::new();
    if let Some(ros_args) = n.attribute("ros_args").or(n.attribute("ros-args")) {
        let toks: Vec<&str> = ros_args.split_whitespace().collect();
        if let Some(i) = toks.iter().position(|t| *t == "--enclave" || *t == "-e") {
            enclave = toks.get(i + 1).map(|s| sub(s));
        }
    }
    if let Some(args) = n.attribute("args") {
        arguments.extend(args.split_whitespace().map(sub));
    }
    LaunchNodeDecl {
        kind,
        pkg: sub(n.attribute("pkg").unwrap_or_default()),
        exec: sub(n.attribute("exec").unwrap_or_default()),
        plugin: n.attribute("plugin").map(sub),
        name: n.attribute("name").map(sub),
        namespace: n.attribute("namespace").or(n.attribute("ns")).map(sub),
        params,
        remaps,
        enclave,
        arguments,
        conditional,
        loc: Loc::new(file, r.start, r.end),
    }
}

// ============================================================================ YAML

pub fn parse_yaml(file: FileId, rel_path: &str, text: &str) -> Result<LaunchFile, String> {
    let docs = yaml::parse(text)?;
    let dir = dir_of(rel_path);
    let Some(list) = docs.first().and_then(|d| d.get("launch")) else {
        return Err("no top-level `launch:` key".into());
    };
    let mut partial = false;
    let items = yaml_items(file, &dir, list, &mut partial);
    Ok(LaunchFile {
        file,
        items,
        partial,
    })
}

fn ystr(n: Option<&YNode>, dir: &str) -> Option<LVal> {
    n.and_then(|v| v.as_str()).map(|s| parse_subst(s, dir))
}

fn yaml_items(file: FileId, dir: &str, list: &YNode, partial: &mut bool) -> Vec<LaunchItem> {
    let mut items = Vec::new();
    for entry in list.items() {
        for (k, v) in entry.entries() {
            let loc = Loc::new(file, k.start(), v.end());
            let conditional = v.get("if").is_some() || v.get("unless").is_some();
            match k.as_str().unwrap_or("") {
                "arg" | "let" => items.push(LaunchItem::Arg {
                    name: v
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    default: ystr(v.get("default").or(v.get("value")), dir),
                    loc,
                }),
                "node" => {
                    let mut params = Vec::new();
                    for p in v.get("param").map(|p| p.items()).unwrap_or(&[]) {
                        let ploc = Loc::new(file, p.start(), p.end());
                        if let Some(from) = ystr(p.get("from"), dir) {
                            params.push(LaunchParamSource::File {
                                path: from,
                                root_key: None,
                                loc: ploc,
                            });
                        } else if let Some(name) = p.get("name").and_then(|n| n.as_str()) {
                            let val = p.get("value");
                            params.push(LaunchParamSource::Inline {
                                name: name.to_string(),
                                value: ystr(val, dir).unwrap_or(LVal::Unknown),
                                typed: val.map(|v| v.to_param_value()),
                                loc: ploc,
                            });
                        }
                    }
                    let remaps = v
                        .get("remap")
                        .map(|r| r.items())
                        .unwrap_or(&[])
                        .iter()
                        .map(|r| {
                            (
                                ystr(r.get("from"), dir).unwrap_or(LVal::Unknown),
                                ystr(r.get("to"), dir).unwrap_or(LVal::Unknown),
                                Loc::new(file, r.start(), r.end()),
                            )
                        })
                        .collect();
                    items.push(LaunchItem::Node(LaunchNodeDecl {
                        kind: LaunchNodeKind::Node,
                        pkg: ystr(v.get("pkg"), dir).unwrap_or(LVal::Unknown),
                        exec: ystr(v.get("exec"), dir).unwrap_or(LVal::Unknown),
                        plugin: None,
                        name: ystr(v.get("name"), dir),
                        namespace: ystr(v.get("namespace").or(v.get("ns")), dir),
                        params,
                        remaps,
                        enclave: None,
                        arguments: Vec::new(),
                        conditional,
                        loc,
                    }));
                }
                "include" => {
                    let args = v
                        .get("arg")
                        .map(|a| a.items())
                        .unwrap_or(&[])
                        .iter()
                        .filter_map(|a| {
                            Some((
                                a.get("name")?.as_str()?.to_string(),
                                ystr(a.get("value"), dir).unwrap_or(LVal::Unknown),
                            ))
                        })
                        .collect();
                    items.push(LaunchItem::Include {
                        path: ystr(v.get("file"), dir).unwrap_or(LVal::Unknown),
                        args,
                        conditional,
                        loc,
                    });
                }
                "group" => {
                    let mut ns = None;
                    let mut children = Vec::new();
                    if let Some(ch) = v.get("children") {
                        for c in ch.items() {
                            if let Some(p) =
                                c.get("push-ros-namespace").or(c.get("push_ros_namespace"))
                            {
                                ns = ystr(p.get("namespace"), dir);
                            }
                        }
                        children = yaml_items(file, dir, ch, partial);
                    }
                    let scoped = v.get("scoped").and_then(|s| s.as_str()) != Some("false");
                    if conditional {
                        children = mark_conditional(children);
                    }
                    items.push(LaunchItem::Group {
                        namespace: ns,
                        items: children,
                        scoped,
                        loc,
                    });
                }
                "set_parameter" | "set-parameter" => items.push(LaunchItem::SetParam {
                    name: v
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    value: ystr(v.get("value"), dir).unwrap_or(LVal::Unknown),
                    loc,
                }),
                "set_remap" | "set-remap" => items.push(LaunchItem::SetRemap {
                    from: ystr(v.get("from"), dir).unwrap_or(LVal::Unknown),
                    to: ystr(v.get("to"), dir).unwrap_or(LVal::Unknown),
                    loc,
                }),
                "push-ros-namespace" | "push_ros_namespace" => {}
                _ => *partial = true,
            }
        }
    }
    items
}

// ============================================================================ Python

#[derive(Debug, Clone)]
enum PyVal {
    S(LVal),
    Bool(bool),
    Int(i64),
    Float(f64),
    List(Vec<PyVal>),
    Dict(Vec<(PyVal, PyVal)>),
    Item(Box<LaunchItem>),
    /// A launch description: a list of items.
    Desc(Vec<LaunchItem>),
    /// `RewrittenYaml(source_file=…, root_key=…)` / `ParameterFile(…)`.
    ParamFile {
        path: LVal,
        root_key: Option<LVal>,
    },
    /// A launch description source wrapping a path.
    Source(LVal),
    Condition,
    Unknown,
}

impl PyVal {
    fn to_lval(&self) -> LVal {
        match self {
            PyVal::S(v) => v.clone(),
            PyVal::Bool(b) => LVal::Lit(if *b { "true".into() } else { "false".into() }),
            PyVal::Int(i) => LVal::Lit(i.to_string()),
            PyVal::Float(f) => LVal::Lit(f.to_string()),
            PyVal::Source(p) => p.clone(),
            PyVal::List(l) => LVal::Concat(l.iter().map(|v| v.to_lval()).collect()),
            _ => LVal::Unknown,
        }
    }

    fn typed(&self) -> Option<ParamValue> {
        match self {
            PyVal::S(LVal::Lit(s)) => Some(ParamValue::Str(s.clone())),
            PyVal::Bool(b) => Some(ParamValue::Bool(*b)),
            PyVal::Int(i) => Some(ParamValue::Int(*i)),
            PyVal::Float(f) => Some(ParamValue::Double(*f)),
            PyVal::List(l) => {
                if l.is_empty() {
                    return Some(ParamValue::EmptyArray);
                }
                let vals: Option<Vec<ParamValue>> = l.iter().map(|v| v.typed()).collect();
                let vals = vals?;
                if let Some(v) = vals
                    .iter()
                    .map(|v| {
                        if let ParamValue::Double(d) = v {
                            Some(*d)
                        } else {
                            None
                        }
                    })
                    .collect::<Option<Vec<_>>>()
                {
                    return Some(ParamValue::DoubleArray(v));
                }
                if let Some(v) = vals
                    .iter()
                    .map(|v| {
                        if let ParamValue::Int(d) = v {
                            Some(*d)
                        } else {
                            None
                        }
                    })
                    .collect::<Option<Vec<_>>>()
                {
                    return Some(ParamValue::IntArray(v));
                }
                if let Some(v) = vals
                    .iter()
                    .map(|v| {
                        if let ParamValue::Str(d) = v {
                            Some(d.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Option<Vec<_>>>()
                {
                    return Some(ParamValue::StrArray(v));
                }
                if let Some(v) = vals
                    .iter()
                    .map(|v| {
                        if let ParamValue::Bool(d) = v {
                            Some(*d)
                        } else {
                            None
                        }
                    })
                    .collect::<Option<Vec<_>>>()
                {
                    return Some(ParamValue::BoolArray(v));
                }
                None
            }
            _ => None,
        }
    }
}

struct PyCtx<'a> {
    file: FileId,
    dir: String,
    funcs: HashMap<String, &'a ast::StmtFunctionDef>,
    partial: bool,
    depth: usize,
}

pub fn parse_python(file: FileId, rel_path: &str, text: &str) -> Result<LaunchFile, String> {
    let module = rustpython_parser::parse(text, rustpython_parser::Mode::Module, rel_path)
        .map_err(|e| e.to_string())?;
    let ast::Mod::Module(m) = module else {
        return Err("not a module".into());
    };
    let mut funcs = HashMap::new();
    for s in &m.body {
        if let Stmt::FunctionDef(f) = s {
            funcs.insert(f.name.to_string(), f);
        }
    }
    let mut ctx = PyCtx {
        file,
        dir: dir_of(rel_path),
        funcs,
        partial: false,
        depth: 0,
    };
    let mut env: HashMap<String, PyVal> = HashMap::new();
    // Module-level constants (e.g. package names).
    for s in &m.body {
        if let Stmt::Assign(_) = s {
            let mut items = Vec::new();
            exec_stmt(&mut ctx, s, &mut env, &mut items, false);
        }
    }
    let Some(gld) = ctx.funcs.get("generate_launch_description").copied() else {
        return Err("no generate_launch_description()".into());
    };
    let mut items = Vec::new();
    let ret = exec_body(&mut ctx, &gld.body, &mut env, &mut items, false);
    if let Some(PyVal::Desc(d)) = ret {
        items.extend(d);
    }
    Ok(LaunchFile {
        file,
        items,
        partial: ctx.partial,
    })
}

/// Executes statements; returns the value of a `return`, if reached.
fn exec_body(
    ctx: &mut PyCtx,
    body: &[Stmt],
    env: &mut HashMap<String, PyVal>,
    added: &mut Vec<LaunchItem>,
    cond: bool,
) -> Option<PyVal> {
    for s in body {
        if let Some(r) = exec_stmt(ctx, s, env, added, cond) {
            return Some(r);
        }
    }
    None
}

fn exec_stmt(
    ctx: &mut PyCtx,
    s: &Stmt,
    env: &mut HashMap<String, PyVal>,
    added: &mut Vec<LaunchItem>,
    cond: bool,
) -> Option<PyVal> {
    match s {
        Stmt::Assign(a) => {
            let v = eval(ctx, &a.value, env, added, cond);
            for t in &a.targets {
                match t {
                    Expr::Name(n) => {
                        env.insert(n.id.to_string(), v.clone());
                    }
                    Expr::Tuple(tp) => {
                        for e in &tp.elts {
                            if let Expr::Name(n) = e {
                                env.insert(n.id.to_string(), PyVal::Unknown);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Stmt::AnnAssign(a) => {
            if let (Expr::Name(n), Some(v)) = (&*a.target, &a.value) {
                let v = eval(ctx, v, env, added, cond);
                env.insert(n.id.to_string(), v);
            }
        }
        Stmt::AugAssign(a) => {
            if let Expr::Name(n) = &*a.target {
                let rhs = eval(ctx, &a.value, env, added, cond);
                if let (Some(PyVal::List(l)), PyVal::List(r)) = (env.get_mut(n.id.as_str()), rhs) {
                    l.extend(r);
                }
            }
        }
        Stmt::Expr(e) => {
            eval(ctx, &e.value, env, added, cond);
        }
        Stmt::Return(r) => {
            return Some(
                r.value
                    .as_ref()
                    .map_or(PyVal::Unknown, |v| eval(ctx, v, env, added, cond)),
            );
        }
        Stmt::If(i) => {
            // Runtime conditions: both branches may run; their nodes are conditional.
            let mut env2 = env.clone();
            let r1 = exec_body(ctx, &i.body, env, added, true);
            let r2 = exec_body(ctx, &i.orelse, &mut env2, added, true);
            for (k, v) in env2 {
                env.entry(k).or_insert(v);
            }
            if r1.is_some() || r2.is_some() {
                ctx.partial = true;
                return r1.or(r2);
            }
        }
        Stmt::For(f) => {
            ctx.partial = true;
            if let Expr::Name(n) = &*f.target {
                env.insert(n.id.to_string(), PyVal::Unknown);
            }
            exec_body(ctx, &f.body, env, added, true);
        }
        Stmt::With(w) => {
            exec_body(ctx, &w.body, env, added, cond);
        }
        Stmt::Try(t) => {
            exec_body(ctx, &t.body, env, added, cond);
        }
        Stmt::FunctionDef(_) | Stmt::Import(_) | Stmt::ImportFrom(_) | Stmt::Pass(_) => {}
        _ => {}
    }
    None
}

fn call_name(func: &Expr) -> (String, Option<&Expr>) {
    match func {
        Expr::Name(n) => (n.id.to_string(), None),
        Expr::Attribute(a) => (a.attr.to_string(), Some(&*a.value)),
        _ => (String::new(), None),
    }
}

fn kw<'a>(c: &'a ast::ExprCall, name: &str) -> Option<&'a Expr> {
    c.keywords
        .iter()
        .find(|k| k.arg.as_ref().map(|a| a.as_str()) == Some(name))
        .map(|k| &k.value)
}

fn arg_or_kw<'a>(c: &'a ast::ExprCall, idx: usize, name: &str) -> Option<&'a Expr> {
    kw(c, name).or_else(|| c.args.get(idx))
}

fn eval(
    ctx: &mut PyCtx,
    e: &Expr,
    env: &mut HashMap<String, PyVal>,
    added: &mut Vec<LaunchItem>,
    cond: bool,
) -> PyVal {
    match e {
        Expr::Constant(c) => match &c.value {
            Constant::Str(s) => PyVal::S(LVal::Lit(s.clone())),
            Constant::Bool(b) => PyVal::Bool(*b),
            Constant::Int(i) => i
                .to_string()
                .parse()
                .map(PyVal::Int)
                .unwrap_or(PyVal::Unknown),
            Constant::Float(f) => PyVal::Float(*f),
            _ => PyVal::Unknown,
        },
        Expr::Name(n) => match n.id.as_str() {
            "__file__" => PyVal::S(LVal::Lit(format!("{WS_PREFIX}{}/__file__", ctx.dir))),
            "True" => PyVal::Bool(true),
            "False" => PyVal::Bool(false),
            id => env.get(id).cloned().unwrap_or(PyVal::Unknown),
        },
        Expr::List(l) => PyVal::List(
            l.elts
                .iter()
                .map(|x| eval(ctx, x, env, added, cond))
                .collect(),
        ),
        Expr::Tuple(t) => PyVal::List(
            t.elts
                .iter()
                .map(|x| eval(ctx, x, env, added, cond))
                .collect(),
        ),
        Expr::Dict(d) => PyVal::Dict(
            d.keys
                .iter()
                .zip(&d.values)
                .filter_map(|(k, v)| {
                    Some((
                        eval(ctx, k.as_ref()?, env, added, cond),
                        eval(ctx, v, env, added, cond),
                    ))
                })
                .collect(),
        ),
        Expr::BinOp(b) if matches!(b.op, ast::Operator::Add) => {
            let l = eval(ctx, &b.left, env, added, cond);
            let r = eval(ctx, &b.right, env, added, cond);
            match (l, r) {
                (PyVal::List(mut a), PyVal::List(b)) => {
                    a.extend(b);
                    PyVal::List(a)
                }
                (a, b) => PyVal::S(simplify(LVal::Concat(vec![a.to_lval(), b.to_lval()]))),
            }
        }
        // `PathSubstitution(FindPackageShare("p")) / "config" / "x.yaml"`, `pathlib.Path(..) / ..`
        Expr::BinOp(b) if matches!(b.op, ast::Operator::Div) => {
            match (
                eval(ctx, &b.left, env, added, cond),
                eval(ctx, &b.right, env, added, cond),
            ) {
                (PyVal::S(LVal::PathJoin(mut parts)), PyVal::S(r)) => {
                    parts.push(r);
                    PyVal::S(simplify(LVal::PathJoin(parts)))
                }
                (PyVal::S(l), PyVal::S(r)) => PyVal::S(simplify(LVal::PathJoin(vec![l, r]))),
                _ => PyVal::Unknown, // numbers, or anything we cannot evaluate
            }
        }
        Expr::JoinedStr(j) => {
            let parts = j
                .values
                .iter()
                .map(|v| match v {
                    Expr::FormattedValue(fv) => eval(ctx, &fv.value, env, added, cond).to_lval(),
                    other => eval(ctx, other, env, added, cond).to_lval(),
                })
                .collect();
            PyVal::S(simplify(LVal::Concat(parts)))
        }
        Expr::Subscript(_) | Expr::IfExp(_) => PyVal::Unknown,
        Expr::Call(c) => eval_call(ctx, c, env, added, cond),
        _ => PyVal::Unknown,
    }
}

fn str_of(
    ctx: &mut PyCtx,
    e: Option<&Expr>,
    env: &mut HashMap<String, PyVal>,
    added: &mut Vec<LaunchItem>,
    cond: bool,
) -> Option<LVal> {
    e.map(|x| eval(ctx, x, env, added, cond).to_lval())
}

fn eval_call(
    ctx: &mut PyCtx,
    c: &ast::ExprCall,
    env: &mut HashMap<String, PyVal>,
    added: &mut Vec<LaunchItem>,
    cond: bool,
) -> PyVal {
    let (name, recv) = call_name(&c.func);
    let loc = Loc::new(
        ctx.file,
        c.range().start().to_usize(),
        c.range().end().to_usize(),
    );
    let is_cond = cond || kw(c, "condition").is_some();
    match name.as_str() {
        "LaunchConfiguration" => match c
            .args
            .first()
            .or(kw(c, "variable_name"))
            .map(|a| eval(ctx, a, env, added, cond))
        {
            Some(PyVal::S(LVal::Lit(n))) => PyVal::S(LVal::Arg(n)),
            _ => PyVal::Unknown,
        },
        "TextSubstitution" => {
            kw(c, "text").map_or(PyVal::Unknown, |t| eval(ctx, t, env, added, cond))
        }
        "FindPackageShare" | "get_package_share_directory" | "get_package_share_path" => {
            match arg_or_kw(c, 0, "package").map(|a| eval(ctx, a, env, added, cond)) {
                Some(PyVal::S(LVal::Lit(p))) => PyVal::S(LVal::PkgShare(p)),
                _ => PyVal::Unknown,
            }
        }
        "FindPackagePrefix" | "get_package_prefix" => {
            match arg_or_kw(c, 0, "package").map(|a| eval(ctx, a, env, added, cond)) {
                Some(PyVal::S(LVal::Lit(p))) => PyVal::S(LVal::PkgPrefix(p)),
                _ => PyVal::Unknown,
            }
        }
        // The directory of the current launch file (same encoding as `dirname(__file__)`).
        "ThisLaunchFileDir" if c.args.is_empty() => {
            PyVal::S(LVal::Lit(format!("{WS_PREFIX}{}", ctx.dir)))
        }
        "PathSubstitution" | "Path" | "PurePath" if c.args.len() == 1 => {
            eval(ctx, &c.args[0], env, added, cond)
        }
        "PathJoinSubstitution" => match c.args.first().map(|a| eval(ctx, a, env, added, cond)) {
            Some(PyVal::List(l)) => PyVal::S(simplify(LVal::PathJoin(
                l.iter().map(|v| v.to_lval()).collect(),
            ))),
            _ => PyVal::Unknown,
        },
        "join"
            if recv
                .is_some_and(|r| matches!(r, Expr::Attribute(a) if a.attr.as_str() == "path")) =>
        {
            let parts: Vec<LVal> = c
                .args
                .iter()
                .map(|a| eval(ctx, a, env, added, cond).to_lval())
                .collect();
            PyVal::S(simplify(LVal::PathJoin(parts)))
        }
        "dirname" | "realpath" | "abspath" => {
            match c.args.first().map(|a| eval(ctx, a, env, added, cond)) {
                Some(PyVal::S(LVal::Lit(p))) if name == "dirname" => {
                    PyVal::S(LVal::Lit(dir_of(&p)))
                }
                Some(v) if name != "dirname" => v,
                _ => PyVal::Unknown,
            }
        }
        "Path" => c
            .args
            .first()
            .map_or(PyVal::Unknown, |a| eval(ctx, a, env, added, cond)),
        "str" => c
            .args
            .first()
            .map_or(PyVal::Unknown, |a| eval(ctx, a, env, added, cond)),
        "PythonLaunchDescriptionSource"
        | "XMLLaunchDescriptionSource"
        | "YAMLLaunchDescriptionSource"
        | "AnyLaunchDescriptionSource"
        | "FrontendLaunchDescriptionSource" => {
            let p = arg_or_kw(c, 0, "launch_file_path").map(|a| eval(ctx, a, env, added, cond));
            match p {
                Some(PyVal::List(l)) => PyVal::Source(simplify(LVal::Concat(
                    l.iter().map(|v| v.to_lval()).collect(),
                ))),
                Some(v) => PyVal::Source(v.to_lval()),
                None => PyVal::Unknown,
            }
        }
        "IncludeLaunchDescription" => {
            let src = arg_or_kw(c, 0, "launch_description_source")
                .map(|a| eval(ctx, a, env, added, cond));
            let path = match src {
                Some(PyVal::Source(p)) => p,
                Some(PyVal::S(p)) => p,
                _ => LVal::Unknown,
            };
            let mut args = Vec::new();
            if let Some(la) = kw(c, "launch_arguments") {
                // `{...}.items()` or a list of tuples
                let target = match la {
                    Expr::Call(ic) if call_name(&ic.func).0 == "items" => match &*ic.func {
                        Expr::Attribute(a) => eval(ctx, &a.value, env, added, cond),
                        _ => PyVal::Unknown,
                    },
                    other => eval(ctx, other, env, added, cond),
                };
                match target {
                    PyVal::Dict(d) => {
                        for (k, v) in d {
                            if let PyVal::S(LVal::Lit(k)) = k {
                                args.push((k, v.to_lval()));
                            }
                        }
                    }
                    PyVal::List(l) => {
                        for t in l {
                            if let PyVal::List(kv) = t {
                                if let (Some(PyVal::S(LVal::Lit(k))), Some(v)) =
                                    (kv.first(), kv.get(1))
                                {
                                    args.push((k.clone(), v.to_lval()));
                                }
                            }
                        }
                    }
                    _ => ctx.partial = true,
                }
            }
            PyVal::Item(Box::new(LaunchItem::Include {
                path,
                args,
                conditional: is_cond,
                loc,
            }))
        }
        "Node" | "LifecycleNode" | "ComposableNode" | "ComposableNodeContainer" => {
            let kind = match name.as_str() {
                "LifecycleNode" => LaunchNodeKind::LifecycleNode,
                "ComposableNode" => LaunchNodeKind::Composable,
                "ComposableNodeContainer" => LaunchNodeKind::Container,
                _ => LaunchNodeKind::Node,
            };
            let decl = py_node_decl(ctx, c, kind, env, added, is_cond, loc);
            if kind == LaunchNodeKind::Container {
                let mut items = vec![LaunchItem::Node(decl)];
                if let Some(PyVal::List(l)) =
                    kw(c, "composable_node_descriptions").map(|a| eval(ctx, a, env, added, cond))
                {
                    for v in l {
                        if let PyVal::Item(i) = v {
                            items.push(*i);
                        }
                    }
                }
                // A conditional container loads its components only on that branch.
                return PyVal::Desc(if is_cond {
                    mark_conditional(items)
                } else {
                    items
                });
            }
            PyVal::Item(Box::new(LaunchItem::Node(decl)))
        }
        "LoadComposableNodes" => {
            match kw(c, "composable_node_descriptions").map(|a| eval(ctx, a, env, added, cond)) {
                Some(PyVal::List(l)) => {
                    let items: Vec<LaunchItem> = l
                        .into_iter()
                        .filter_map(|v| {
                            if let PyVal::Item(i) = v {
                                Some(*i)
                            } else {
                                None
                            }
                        })
                        .collect();
                    PyVal::Desc(if is_cond {
                        mark_conditional(items)
                    } else {
                        items
                    })
                }
                _ => PyVal::Unknown,
            }
        }
        "DeclareLaunchArgument" => {
            let n = match arg_or_kw(c, 0, "name").map(|a| eval(ctx, a, env, added, cond)) {
                Some(PyVal::S(LVal::Lit(n))) => n,
                _ => return PyVal::Unknown,
            };
            let default = kw(c, "default_value").map(|a| eval(ctx, a, env, added, cond).to_lval());
            PyVal::Item(Box::new(LaunchItem::Arg {
                name: n,
                default,
                loc,
            }))
        }
        "SetLaunchConfiguration" => {
            let n = match arg_or_kw(c, 0, "name").map(|a| eval(ctx, a, env, added, cond)) {
                Some(PyVal::S(LVal::Lit(n))) => n,
                _ => return PyVal::Unknown,
            };
            let v = arg_or_kw(c, 1, "value").map(|a| eval(ctx, a, env, added, cond).to_lval());
            PyVal::Item(Box::new(LaunchItem::Arg {
                name: n,
                default: v,
                loc,
            }))
        }
        "PushRosNamespace" | "PushROSNamespace" => {
            let ns = arg_or_kw(c, 0, "namespace").map(|a| eval(ctx, a, env, added, cond).to_lval());
            PyVal::Item(Box::new(LaunchItem::Group {
                namespace: ns,
                items: Vec::new(),
                scoped: false,
                loc,
            }))
        }
        "SetParameter" => {
            let n = match arg_or_kw(c, 0, "name").map(|a| eval(ctx, a, env, added, cond)) {
                Some(PyVal::S(LVal::Lit(n))) => n,
                _ => return PyVal::Unknown,
            };
            let v = arg_or_kw(c, 1, "value")
                .map_or(LVal::Unknown, |a| eval(ctx, a, env, added, cond).to_lval());
            PyVal::Item(Box::new(LaunchItem::SetParam {
                name: n,
                value: v,
                loc,
            }))
        }
        "SetRemap" => {
            let f = str_of(ctx, arg_or_kw(c, 0, "src"), env, added, cond).unwrap_or(LVal::Unknown);
            let t = str_of(ctx, arg_or_kw(c, 1, "dst"), env, added, cond).unwrap_or(LVal::Unknown);
            PyVal::Item(Box::new(LaunchItem::SetRemap {
                from: f,
                to: t,
                loc,
            }))
        }
        "SetEnvironmentVariable" => {
            let n = match arg_or_kw(c, 0, "name").map(|a| eval(ctx, a, env, added, cond)) {
                Some(PyVal::S(LVal::Lit(n))) => n,
                _ => return PyVal::Unknown,
            };
            let v = arg_or_kw(c, 1, "value")
                .map_or(LVal::Unknown, |a| eval(ctx, a, env, added, cond).to_lval());
            PyVal::Item(Box::new(LaunchItem::SetEnv {
                name: n,
                value: v,
                loc,
            }))
        }
        "GroupAction" => {
            let acts = arg_or_kw(c, 0, "actions").map(|a| eval(ctx, a, env, added, is_cond));
            let mut items = Vec::new();
            let mut ns = None;
            if let Some(PyVal::List(l)) = acts {
                for v in l {
                    collect_items(ctx, v, &mut items);
                }
            }
            // A PushRosNamespace at the start of a group names the group's namespace.
            if let Some(LaunchItem::Group {
                namespace,
                items: inner,
                scoped: false,
                ..
            }) = items.first()
            {
                if inner.is_empty() {
                    ns = namespace.clone();
                    items.remove(0);
                }
            }
            let scoped = !matches!(
                kw(c, "scoped").map(|a| eval(ctx, a, env, added, cond)),
                Some(PyVal::Bool(false))
            );
            let items = if is_cond {
                mark_conditional(items)
            } else {
                items
            };
            PyVal::Item(Box::new(LaunchItem::Group {
                namespace: ns,
                items,
                scoped,
                loc,
            }))
        }
        "LaunchDescription" => {
            let mut items = Vec::new();
            if let Some(PyVal::List(l)) = c.args.first().map(|a| eval(ctx, a, env, added, cond)) {
                for v in l {
                    collect_items(ctx, v, &mut items);
                }
            }
            PyVal::Desc(items)
        }
        "RewrittenYaml" | "ParameterFile" => {
            let inner = arg_or_kw(c, 0, "source_file")
                .or(kw(c, "param_file"))
                .map(|a| eval(ctx, a, env, added, cond));
            // ParameterFile(RewrittenYaml(...)) keeps the inner file and root key.
            if let Some(PyVal::ParamFile { path, root_key }) = inner {
                let rk = if name == "RewrittenYaml" {
                    str_of(ctx, kw(c, "root_key"), env, added, cond).or(root_key)
                } else {
                    root_key
                };
                return PyVal::ParamFile { path, root_key: rk };
            }
            let path = inner.map(|v| v.to_lval()).unwrap_or(LVal::Unknown);
            let root_key = if name == "RewrittenYaml" {
                str_of(ctx, kw(c, "root_key"), env, added, cond)
            } else {
                None
            };
            PyVal::ParamFile { path, root_key }
        }
        "IfCondition"
        | "UnlessCondition"
        | "LaunchConfigurationEquals"
        | "LaunchConfigurationNotEquals"
        | "PythonExpression"
        | "EqualsSubstitution" => PyVal::Condition,
        "add_action" | "add_entity" => {
            if let Some(a) = c.args.first() {
                let v = eval(ctx, a, env, added, cond);
                let mut items = Vec::new();
                collect_items(ctx, v, &mut items);
                if cond {
                    items = mark_conditional(items);
                }
                added.extend(items);
            }
            PyVal::Unknown
        }
        "append" | "extend" => {
            // list.append(x) on a local list
            if let (Some(Expr::Name(n)), Some(a)) = (recv, c.args.first()) {
                let v = eval(ctx, a, env, added, cond);
                if let Some(PyVal::List(l)) = env.get_mut(n.id.as_str()) {
                    if name == "append" {
                        l.push(v);
                    } else if let PyVal::List(more) = v {
                        l.extend(more);
                    }
                } else if let Some(PyVal::Desc(d)) = env.get_mut(n.id.as_str()) {
                    let mut items = Vec::new();
                    collect_items(ctx, v, &mut items);
                    d.extend(items);
                }
            }
            PyVal::Unknown
        }
        "OpaqueFunction"
        | "TimerAction"
        | "RegisterEventHandler"
        | "ExecuteProcess"
        | "Shutdown"
        | "LogInfo"
        | "EmitEvent" => {
            if name == "OpaqueFunction" || name == "TimerAction" {
                ctx.partial = true;
                if name == "TimerAction" {
                    // TimerAction(period=…, actions=[…]) still launches its actions.
                    if let Some(PyVal::List(l)) =
                        kw(c, "actions").map(|a| eval(ctx, a, env, added, true))
                    {
                        let mut items = Vec::new();
                        for v in l {
                            collect_items(ctx, v, &mut items);
                        }
                        return PyVal::Desc(mark_conditional(items));
                    }
                }
            }
            PyVal::Unknown
        }
        "items" => PyVal::Unknown,
        _ => {
            // A helper function defined in this file: evaluate it with bound args.
            if recv.is_none() {
                if let Some(f) = ctx.funcs.get(name.as_str()).copied() {
                    if ctx.depth < 4 {
                        let mut local: HashMap<String, PyVal> = env.clone();
                        for (i, p) in f.args.args.iter().enumerate() {
                            let v = c
                                .args
                                .get(i)
                                .map(|a| eval(ctx, a, env, added, cond))
                                .or_else(|| {
                                    kw(c, p.def.arg.as_str())
                                        .map(|a| eval(ctx, a, env, added, cond))
                                })
                                .or_else(|| {
                                    p.default.as_ref().map(|d| eval(ctx, d, env, added, cond))
                                })
                                .unwrap_or(PyVal::Unknown);
                            local.insert(p.def.arg.to_string(), v);
                        }
                        ctx.depth += 1;
                        let mut inner_added = Vec::new();
                        let ret = exec_body(ctx, &f.body, &mut local, &mut inner_added, cond);
                        ctx.depth -= 1;
                        added.extend(inner_added);
                        return ret.unwrap_or(PyVal::Unknown);
                    }
                }
            }
            PyVal::Unknown
        }
    }
}

fn mark_conditional(items: Vec<LaunchItem>) -> Vec<LaunchItem> {
    items
        .into_iter()
        .map(|i| match i {
            LaunchItem::Node(mut n) => {
                n.conditional = true;
                LaunchItem::Node(n)
            }
            LaunchItem::Include {
                path, args, loc, ..
            } => LaunchItem::Include {
                path,
                args,
                conditional: true,
                loc,
            },
            LaunchItem::Group {
                namespace,
                items,
                scoped,
                loc,
            } => LaunchItem::Group {
                namespace,
                items: mark_conditional(items),
                scoped,
                loc,
            },
            other => other,
        })
        .collect()
}

fn collect_items(ctx: &mut PyCtx, v: PyVal, out: &mut Vec<LaunchItem>) {
    match v {
        PyVal::Item(i) => out.push(*i),
        PyVal::Desc(d) => out.extend(d),
        PyVal::List(l) => {
            for x in l {
                collect_items(ctx, x, out);
            }
        }
        PyVal::Condition => {}
        _ => ctx.partial = true,
    }
}

#[allow(clippy::too_many_arguments)]
fn py_node_decl(
    ctx: &mut PyCtx,
    c: &ast::ExprCall,
    kind: LaunchNodeKind,
    env: &mut HashMap<String, PyVal>,
    added: &mut Vec<LaunchItem>,
    cond: bool,
    loc: Loc,
) -> LaunchNodeDecl {
    let get = |ctx: &mut PyCtx,
               env: &mut HashMap<String, PyVal>,
               added: &mut Vec<LaunchItem>,
               k: &str|
     -> Option<LVal> { kw(c, k).map(|a| eval(ctx, a, env, added, cond).to_lval()) };
    let pkg = get(ctx, env, added, "package").unwrap_or(LVal::Unknown);
    let exec = get(ctx, env, added, "executable")
        .or_else(|| get(ctx, env, added, "node_executable"))
        .unwrap_or(LVal::Unknown);
    let plugin = get(ctx, env, added, "plugin");
    let name = get(ctx, env, added, "name").or_else(|| get(ctx, env, added, "node_name"));
    let namespace =
        get(ctx, env, added, "namespace").or_else(|| get(ctx, env, added, "node_namespace"));
    let mut params = Vec::new();
    if let Some(p) = kw(c, "parameters") {
        let ploc = Loc::new(
            ctx.file,
            p.range().start().to_usize(),
            p.range().end().to_usize(),
        );
        match eval(ctx, p, env, added, cond) {
            PyVal::List(l) => {
                for v in l {
                    push_param(&mut params, v, ploc);
                }
            }
            _ => params.push(LaunchParamSource::Opaque { loc: ploc }),
        }
    }
    let mut remaps = Vec::new();
    if let Some(r) = kw(c, "remappings") {
        let rloc = Loc::new(
            ctx.file,
            r.range().start().to_usize(),
            r.range().end().to_usize(),
        );
        match eval(ctx, r, env, added, cond) {
            PyVal::List(l) => {
                for t in l {
                    if let PyVal::List(kv) = t {
                        if kv.len() == 2 {
                            remaps.push((kv[0].to_lval(), kv[1].to_lval(), rloc));
                        }
                    }
                }
            }
            _ => ctx.partial = true,
        }
    }
    let mut enclave = None;
    let mut arguments = Vec::new();
    for key in ["arguments", "ros_arguments"] {
        if let Some(PyVal::List(l)) = kw(c, key).map(|a| eval(ctx, a, env, added, cond)) {
            let vals: Vec<LVal> = l.iter().map(|v| v.to_lval()).collect();
            if let Some(i) = vals
                .iter()
                .position(|v| matches!(v, LVal::Lit(s) if s == "--enclave" || s == "-e"))
            {
                enclave = vals.get(i + 1).cloned();
            }
            arguments.extend(vals);
        }
    }
    LaunchNodeDecl {
        kind,
        pkg,
        exec,
        plugin,
        name,
        namespace,
        params,
        remaps,
        enclave,
        arguments,
        conditional: cond,
        loc,
    }
}

/// Inline parameter dicts; nested dicts flatten to dotted names (`{'a': {'b': 1}}` → `a.b`).
fn push_dict(params: &mut Vec<LaunchParamSource>, prefix: &str, d: Vec<(PyVal, PyVal)>, loc: Loc) {
    for (k, val) in d {
        let PyVal::S(LVal::Lit(key)) = k else {
            params.push(LaunchParamSource::Opaque { loc });
            continue;
        };
        let name = if prefix.is_empty() {
            key
        } else {
            format!("{prefix}.{key}")
        };
        match val {
            PyVal::Dict(inner) => push_dict(params, &name, inner, loc),
            val => {
                let typed = val.typed();
                params.push(LaunchParamSource::Inline {
                    name,
                    value: val.to_lval(),
                    typed,
                    loc,
                });
            }
        }
    }
}

fn push_param(params: &mut Vec<LaunchParamSource>, v: PyVal, loc: Loc) {
    match v {
        PyVal::Dict(d) => push_dict(params, "", d, loc),
        PyVal::ParamFile { path, root_key } => params.push(LaunchParamSource::File {
            path,
            root_key,
            loc,
        }),
        PyVal::S(path) => params.push(LaunchParamSource::File {
            path,
            root_key: None,
            loc,
        }),
        PyVal::List(l) => {
            for x in l {
                push_param(params, x, loc);
            }
        }
        _ => params.push(LaunchParamSource::Opaque { loc }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes(items: &[LaunchItem]) -> Vec<&LaunchNodeDecl> {
        let mut out = Vec::new();
        for i in items {
            match i {
                LaunchItem::Node(n) => out.push(n),
                LaunchItem::Group { items, .. } => out.extend(nodes(items)),
                _ => {}
            }
        }
        out
    }

    #[test]
    fn subst_parsing() {
        assert_eq!(
            parse_subst("$(find-pkg-share my_pkg)/config/$(var cfg).yaml", ""),
            LVal::Concat(vec![
                LVal::PkgShare("my_pkg".into()),
                LVal::Lit("/config/".into()),
                LVal::Arg("cfg".into()),
                LVal::Lit(".yaml".into())
            ])
        );
    }

    #[test]
    fn xml_launch() {
        let src = r#"<launch>
  <arg name="ns" default="robot1"/>
  <group><push-ros-namespace namespace="$(var ns)"/>
    <node pkg="demo" exec="talker" name="t"><param from="$(find-pkg-share demo)/p.yaml"/><param name="rate" value="10"/><remap from="chatter" to="out"/></node>
  </group>
  <include file="$(find-pkg-share other)/launch/x.launch.py"><arg name="a" value="1"/></include>
</launch>"#;
        let lf = parse_xml(0, "demo/launch/a.launch.xml", src).unwrap();
        let ns = nodes(&lf.items);
        assert_eq!(ns.len(), 1);
        assert_eq!(ns[0].params.len(), 2);
        assert_eq!(ns[0].remaps.len(), 1);
        assert!(matches!(&lf.items[1], LaunchItem::Group { items, .. }
            if matches!(&items[0], LaunchItem::Group { namespace: Some(LVal::Arg(a)), scoped: false, .. } if a == "ns")));
        assert!(matches!(&lf.items[2], LaunchItem::Include { .. }));
    }

    #[test]
    fn python_launch() {
        let src = r#"
import os
from ament_index_python.packages import get_package_share_directory
from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument, IncludeLaunchDescription, GroupAction
from launch.substitutions import LaunchConfiguration
from launch_ros.actions import Node, PushRosNamespace

def make_node(name):
    return Node(package='demo', executable='talker', name=name, parameters=[{'use_sim_time': True, 'rate': 10.0}])

def generate_launch_description():
    use_sim_time = LaunchConfiguration('use_sim_time')
    params = os.path.join(get_package_share_directory('demo'), 'config', 'p.yaml')
    ld = LaunchDescription()
    ld.add_action(DeclareLaunchArgument('use_sim_time', default_value='false'))
    ld.add_action(Node(package='demo', executable='listener', namespace='r1',
                       parameters=[params, {'use_sim_time': use_sim_time}],
                       remappings=[('chatter', '/out')]))
    ld.add_action(make_node('helper'))
    ld.add_action(GroupAction([PushRosNamespace('g'), Node(package='demo', executable='x')]))
    return ld
"#;
        let lf = parse_python(0, "demo/launch/a.launch.py", src).unwrap();
        let ns = nodes(&lf.items);
        assert_eq!(ns.len(), 3, "{:#?}", lf.items);
        assert_eq!(ns[0].exec, LVal::lit("listener"));
        assert!(
            matches!(&ns[0].params[0], LaunchParamSource::File { path: LVal::PathJoin(p), .. } if p[0] == LVal::PkgShare("demo".into()))
        );
        assert!(
            matches!(&ns[0].params[1], LaunchParamSource::Inline { value: LVal::Arg(a), .. } if a == "use_sim_time")
        );
        assert_eq!(ns[1].name, Some(LVal::lit("helper")));
        assert!(matches!(
            &ns[1].params[1],
            LaunchParamSource::Inline {
                typed: Some(ParamValue::Double(_)),
                ..
            }
        ));
        assert!(lf.items.iter().any(
            |i| matches!(i, LaunchItem::Group { namespace: Some(LVal::Lit(g)), .. } if g == "g")
        ));
    }

    /// `IncludeLaunchDescription(PythonLaunchDescriptionSource([ThisLaunchFileDir(), '/b.launch.py']))`
    #[test]
    fn this_launch_file_dir() {
        let src = "from launch import LaunchDescription
from launch.actions import IncludeLaunchDescription
from launch.launch_description_sources import PythonLaunchDescriptionSource
from launch.substitutions import ThisLaunchFileDir
def generate_launch_description():
    return LaunchDescription([IncludeLaunchDescription(PythonLaunchDescriptionSource([ThisLaunchFileDir(), '/b.launch.py']))])
";
        let lf = parse_python(0, "p/launch/a.launch.py", src).unwrap();
        assert!(!lf.partial, "{:#?}", lf.items);
        match &lf.items[0] {
            LaunchItem::Include { path, .. } => {
                let s = format!("{path:?}");
                assert!(s.contains("p/launch") && s.contains("b.launch.py"), "{s}");
            }
            other => panic!("{other:#?}"),
        }
    }

    /// Jazzy-style paths: `PathSubstitution(FindPackageShare("p")) / "config" / "c.yaml"`.
    #[test]
    fn path_substitution_division() {
        let src = "from launch import LaunchDescription
from launch.substitutions import PathSubstitution
from launch_ros.actions import Node
from launch_ros.substitutions import FindPackageShare
def generate_launch_description():
    return LaunchDescription([Node(package='p', executable='x', arguments=['--param-file', PathSubstitution(FindPackageShare('p')) / 'config' / 'c.yaml', 10 / 2])])
";
        let lf = parse_python(0, "p/launch/a.launch.py", src).unwrap();
        let ns = nodes(&lf.items);
        assert_eq!(
            ns[0].arguments[1],
            LVal::PathJoin(vec![
                LVal::PkgShare("p".into()),
                LVal::lit("config"),
                LVal::lit("c.yaml")
            ]),
            "{:#?}",
            ns[0].arguments
        );
    }
}
