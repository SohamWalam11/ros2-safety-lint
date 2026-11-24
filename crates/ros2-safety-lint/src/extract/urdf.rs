//! URDF and a pragmatic xacro subset â†’ [`RobotModel`] (links, joints, `<ros2_control>`).
//!
//! Supported xacro: `property`, `arg`, `${expr}` (property substitution and + - * /
//! arithmetic), `$(arg x)`, `$(find pkg)`, `include`, `macro` (with defaults and
//! `*block` / `**block` params), macro calls, `call`, `insert_block`, `if`/`unless`.
//! Anything else marks the model `partial`, which makes name checks lenient.

use crate::facts::{
    ControlJoint, RobotModel, Ros2ControlBlock, UrdfInertial, UrdfJoint, UrdfLimit, UrdfMesh,
    UrdfNumber,
};
use crate::source::{FileId, Loc};
use roxmltree::{Document, Node, NodeId};
use std::collections::HashMap;
use std::rc::Rc;

/// Resolves `$(find pkg)/rel` / relative include paths to (file id, text).
pub trait Loader<'a> {
    fn load(&self, from_file: FileId, path: &str) -> Option<(FileId, &'a str)>;
}

#[derive(Clone)]
struct Macro {
    doc: usize,
    node: NodeId,
    params: Vec<(String, Option<String>)>,
}

struct X<'a, 'l> {
    loader: &'l dyn Loader<'a>,
    docs: Vec<(FileId, Rc<Document<'a>>)>,
    scopes: Vec<HashMap<String, String>>,
    args: HashMap<String, String>,
    macros: HashMap<String, Macro>,
    blocks: Vec<HashMap<String, Vec<(usize, NodeId)>>>,
    model: RobotModel,
    depth: usize,
    /// Names of properties/parameters whose values are being expanded (cycle guard).
    expanding: Vec<String>,
}

/// The first child element named `tag`.
fn child<'x, 'i>(n: Node<'x, 'i>, tag: &str) -> Option<Node<'x, 'i>> {
    n.children()
        .find(|e| e.is_element() && e.tag_name().name() == tag)
}

fn is_xacro(n: &Node) -> bool {
    n.tag_name()
        .namespace()
        .is_some_and(|ns| ns.contains("xacro"))
}

impl<'a, 'l> X<'a, 'l> {
    fn lookup(&self, name: &str) -> Option<&String> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    /// Evaluates `${â€¦}` and `$(â€¦)` inside an attribute/text value.
    fn eval(&mut self, s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        loop {
            let a = rest.find("${");
            let b = rest.find("$(");
            let (pos, is_expr) = match (a, b) {
                (Some(x), Some(y)) if x < y => (x, true),
                (Some(x), None) => (x, true),
                (_, Some(y)) => (y, false),
                (None, None) => break,
            };
            out.push_str(&rest[..pos]);
            let after = &rest[pos + 2..];
            let close = if is_expr { '}' } else { ')' };
            let Some(end) = after.find(close) else {
                out.push_str(&rest[pos..]);
                rest = "";
                break;
            };
            let inner = after[..end].trim();
            let v = if is_expr {
                self.eval_expr(inner)
            } else {
                self.eval_subst(inner)
            };
            match v {
                Some(v) => out.push_str(&v),
                None => {
                    self.model.partial = true;
                    out.push_str(&rest[pos..pos + 2 + end + 1]);
                }
            }
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        out
    }

    /// Evaluates a numeric attribute without marking the model partial: physical values do
    /// not affect name checks. None when absent, Some(None) when not evaluable.
    fn number(&mut self, attr: Option<&str>) -> UrdfNumber {
        let raw = attr?;
        let partial = self.model.partial;
        let v = self.eval(raw);
        self.model.partial = partial;
        Some(v.trim().parse::<f64>().ok().filter(|x| x.is_finite()))
    }

    /// The `<inertial>` block and mesh references of a link.
    fn link_details(&mut self, file: FileId, link: Node<'_, 'a>, name: &str) {
        if let Some(i) = child(link, "inertial") {
            let mass = child(i, "mass").and_then(|m| self.number(m.attribute("value")));
            let inertia = child(i, "inertia").map(|n| {
                ["ixx", "ixy", "ixz", "iyy", "iyz", "izz"]
                    .iter()
                    .map(|a| self.number(n.attribute(*a)))
                    .collect()
            });
            self.model.inertials.push(UrdfInertial {
                link: name.to_string(),
                loc: Loc::new(file, i.range().start, i.range().end),
                mass,
                inertia,
            });
        }
        for m in link
            .descendants()
            .filter(|e| e.is_element() && e.tag_name().name() == "mesh")
        {
            let Some(a) = m.attribute_node("filename") else {
                continue;
            };
            let partial = self.model.partial;
            let uri = self.eval(a.value());
            self.model.partial = partial;
            let r = a.range_value();
            self.model.meshes.push(UrdfMesh {
                uri,
                loc: Loc::new(file, r.start, r.end),
            });
        }
    }

    fn eval_subst(&mut self, inner: &str) -> Option<String> {
        let (cmd, arg) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
        let arg = arg.trim();
        match cmd {
            "arg" => self.args.get(arg).cloned(),
            "find" => Some(format!("@pkg:{arg}")),
            "optenv" => arg
                .split_once(char::is_whitespace)
                .map(|(_, d)| d.trim().to_string())
                .or(Some(String::new())),
            "eval" => self.eval_expr(arg),
            _ => None,
        }
    }

    fn eval_expr(&mut self, e: &str) -> Option<String> {
        let e = e.trim();
        if let Some(v) = self.lookup(e).cloned() {
            // Values are re-evaluated (lazy properties), but a value that is still being
            // expanded refers to itself, e.g. a forwarded unresolved `parent="${parent}"`.
            if self.expanding.iter().any(|n| n == e) || self.expanding.len() > 64 {
                return None;
            }
            self.expanding.push(e.to_string());
            let r = self.eval(&v);
            self.expanding.pop();
            return Some(r);
        }
        if e.chars().all(|c| c.is_alphanumeric() || c == '_')
            && e.chars().next().is_some_and(|c| c.is_alphabetic())
        {
            return match e {
                "pi" => Some(std::f64::consts::PI.to_string()),
                "True" | "true" => Some("true".into()),
                "False" | "false" => Some("false".into()),
                _ => None,
            };
        }
        // String literal: '...'
        if (e.starts_with('\'') && e.ends_with('\'')) || (e.starts_with('"') && e.ends_with('"')) {
            return Some(e[1..e.len() - 1].to_string());
        }
        let mut p = Arith {
            s: e.as_bytes(),
            i: 0,
            x: self,
        };
        let v = p.expr()?;
        p.skip_ws();
        if p.i != p.s.len() {
            return None;
        }
        Some(if v.fract() == 0.0 && v.abs() < 1e15 {
            format!("{}", v as i64)
        } else {
            format!("{v}")
        })
    }

    fn truthy(&mut self, s: &str) -> Option<bool> {
        let v = self.eval(s);
        match v.trim() {
            "true" | "True" | "1" => Some(true),
            "false" | "False" | "0" => Some(false),
            _ => None,
        }
    }

    fn process(&mut self, doc: usize, id: NodeId, parent_tag: &str) {
        if self.depth > 40 {
            self.model.partial = true;
            return;
        }
        let d = self.docs[doc].1.clone();
        let file = self.docs[doc].0;
        let n = d.get_node(id).expect("node");
        for c in n.children().filter(|c| c.is_element()) {
            self.element(doc, file, &d, c, parent_tag);
        }
    }

    fn element(
        &mut self,
        doc: usize,
        file: FileId,
        d: &Rc<Document<'a>>,
        c: Node<'_, 'a>,
        parent_tag: &str,
    ) {
        let loc = Loc::new(file, c.range().start, c.range().end);
        let tag = c.tag_name().name();
        if is_xacro(&c) {
            match tag {
                "property" => {
                    if let Some(name) = c.attribute("name") {
                        let v = c
                            .attribute("value")
                            .map(|v| self.eval(v))
                            .unwrap_or_default();
                        if let Some(s) = self.scopes.last_mut() {
                            s.insert(name.to_string(), v);
                        }
                    }
                }
                "arg" => {
                    if let Some(name) = c.attribute("name") {
                        let def = c
                            .attribute("default")
                            .map(|v| self.eval(v))
                            .unwrap_or_default();
                        self.args.entry(name.to_string()).or_insert(def);
                    }
                }
                "macro" => {
                    if let Some(name) = c.attribute("name") {
                        let params = c
                            .attribute("params")
                            .unwrap_or("")
                            .split_whitespace()
                            .map(|p| match p.split_once(":=") {
                                Some((k, v)) => (
                                    k.trim_start_matches('^').to_string(),
                                    Some(v.trim_matches(['\'', '"']).to_string()),
                                ),
                                None => match p.split_once('=') {
                                    Some((k, v)) => (
                                        k.to_string(),
                                        Some(v.trim_matches(['\'', '"']).to_string()),
                                    ),
                                    None => (p.to_string(), None),
                                },
                            })
                            .collect();
                        self.macros.insert(
                            name.to_string(),
                            Macro {
                                doc,
                                node: c.id(),
                                params,
                            },
                        );
                    }
                }
                "include" => {
                    let Some(f) = c.attribute("filename") else {
                        return;
                    };
                    let path = self.eval(f);
                    match self.loader.load(file, &path) {
                        Some((fid, text)) => match Document::parse(text) {
                            Ok(nd) => {
                                self.docs.push((fid, Rc::new(nd)));
                                let idx = self.docs.len() - 1;
                                let root = self.docs[idx].1.root_element().id();
                                self.depth += 1;
                                self.process(idx, root, parent_tag);
                                self.depth -= 1;
                            }
                            Err(_) => self.model.partial = true,
                        },
                        None => self.model.partial = true,
                    }
                }
                "if" | "unless" => {
                    let v = c.attribute("value").and_then(|v| self.truthy(v));
                    let run = match (tag, v) {
                        ("if", Some(b)) => b,
                        ("unless", Some(b)) => !b,
                        _ => {
                            self.model.partial = true;
                            true
                        }
                    };
                    if run {
                        self.process(doc, c.id(), parent_tag);
                    }
                }
                "insert_block" => {
                    let Some(name) = c.attribute("name") else {
                        return;
                    };
                    if self.depth > 40 {
                        self.model.partial = true;
                        return;
                    }
                    let Some(blocks) = self.blocks.last().and_then(|b| b.get(name)).cloned() else {
                        return;
                    };
                    // Block content belongs to the caller: evaluate it in the caller's block
                    // and property scope (a forwarded `insert_block` would otherwise resolve
                    // to itself). call_macro pushes one frame of each, so pop exactly one.
                    let frame_b = self.blocks.pop();
                    let frame_s = self.scopes.pop();
                    self.depth += 1;
                    for (bd, bid) in blocks {
                        let dd = self.docs[bd].1.clone();
                        let bf = self.docs[bd].0;
                        if let Some(bn) = dd.get_node(bid) {
                            self.element(bd, bf, &dd, bn, parent_tag);
                        }
                    }
                    self.depth -= 1;
                    self.scopes.extend(frame_s);
                    self.blocks.extend(frame_b);
                }
                "call" => {
                    if let Some(m) = c.attribute("macro") {
                        let m = self.eval(m);
                        self.call_macro(doc, d, c, &m, parent_tag);
                    }
                }
                "element" | "attribute" => self.model.partial = true,
                other => self.call_macro(doc, d, c, other, parent_tag),
            }
            return;
        }
        match tag {
            "link" if parent_tag == "robot" || parent_tag.is_empty() => {
                if let Some(n) = c.attribute("name") {
                    let v = self.eval(n);
                    // Inertia and meshes only when the link's content is plain URDF.
                    if !c.descendants().any(|e| e.is_element() && is_xacro(&e)) {
                        self.link_details(file, c, &v);
                    }
                    self.model.links.push(v);
                }
            }
            "joint" if parent_tag == "robot" || parent_tag.is_empty() => {
                if let Some(n) = c.attribute("name") {
                    let name = self.eval(n);
                    let jt = c
                        .attribute("type")
                        .map(|t| self.eval(t))
                        .unwrap_or_default();
                    let link_of = |tag: &str, this: &mut Self| {
                        c.children()
                            .find(|e| e.is_element() && e.tag_name().name() == tag)
                            .and_then(|e| e.attribute("link"))
                            .map(|l| this.eval(l))
                    };
                    let parent = link_of("parent", self);
                    let child = link_of("child", self);
                    let opaque = c.descendants().any(|e| e.is_element() && is_xacro(&e));
                    let limit = c
                        .children()
                        .find(|e| e.is_element() && e.tag_name().name() == "limit")
                        .map(|l| UrdfLimit {
                            loc: Loc::new(file, l.range().start, l.range().end),
                            lower: self.number(l.attribute("lower")),
                            upper: self.number(l.attribute("upper")),
                            effort: self.number(l.attribute("effort")),
                            velocity: self.number(l.attribute("velocity")),
                        });
                    self.model.joints.push(UrdfJoint {
                        name,
                        joint_type: jt,
                        parent,
                        child,
                        loc,
                        limit,
                        opaque,
                    });
                }
            }
            "ros2_control" => {
                let block = self.control_block(c, loc, file);
                self.model.control.push(block);
            }
            "robot" => self.process(doc, c.id(), "robot"),
            "gazebo" | "transmission" | "material" => {}
            _ => {}
        }
    }

    fn call_macro(
        &mut self,
        doc: usize,
        _d: &Rc<Document<'a>>,
        call: Node<'_, 'a>,
        name: &str,
        parent_tag: &str,
    ) {
        let Some(m) = self.macros.get(name).cloned() else {
            self.model.partial = true;
            return;
        };
        let mut scope = HashMap::new();
        let mut blocks = HashMap::new();
        let child_elems: Vec<Node> = call.children().filter(|c| c.is_element()).collect();
        let mut block_idx = 0;
        for (p, def) in &m.params {
            if let Some(b) = p.strip_prefix("**") {
                if let Some(e) = child_elems.iter().find(|e| e.tag_name().name() == b) {
                    blocks.insert(
                        b.to_string(),
                        e.children()
                            .filter(|x| x.is_element())
                            .map(|x| (doc, x.id()))
                            .collect(),
                    );
                }
            } else if let Some(b) = p.strip_prefix('*') {
                if let Some(e) = child_elems.get(block_idx) {
                    blocks.insert(b.to_string(), vec![(doc, e.id())]);
                }
                block_idx += 1;
            } else {
                let v = call
                    .attribute(p.as_str())
                    .map(|v| self.eval(v))
                    .or_else(|| def.as_ref().map(|d| self.eval(d)));
                match v {
                    Some(v) => {
                        scope.insert(p.clone(), v);
                    }
                    None => self.model.partial = true,
                }
            }
        }
        self.scopes.push(scope);
        self.blocks.push(blocks);
        self.depth += 1;
        self.process(m.doc, m.node, parent_tag);
        self.depth -= 1;
        self.blocks.pop();
        self.scopes.pop();
    }

    fn control_block(&mut self, c: Node<'_, 'a>, loc: Loc, file: FileId) -> Ros2ControlBlock {
        let name = c
            .attribute("name")
            .map(|n| self.eval(n))
            .unwrap_or_default();
        let mut block = Ros2ControlBlock {
            name,
            hardware_plugin: None,
            joints: Vec::new(),
            sensors: Vec::new(),
            gpios: Vec::new(),
            loc,
        };
        // Children may be wrapped in xacro conditionals; flatten one level of xacro:if.
        let mut stack: Vec<Node> = c.children().filter(|x| x.is_element()).collect();
        while let Some(ch) = stack.pop() {
            if is_xacro(&ch) {
                if matches!(ch.tag_name().name(), "if" | "unless") {
                    stack.extend(ch.children().filter(|x| x.is_element()));
                } else {
                    self.model.partial = true;
                }
                continue;
            }
            match ch.tag_name().name() {
                "hardware" => {
                    if let Some(p) = ch.children().find(|x| x.has_tag_name("plugin")) {
                        let t = p.text().unwrap_or("").trim().to_string();
                        block.hardware_plugin = Some(self.eval(&t));
                    } else if ch.children().any(|x| is_xacro(&x)) {
                        block.hardware_plugin = Some(String::from("${unknown}"));
                        self.model.partial = true;
                    }
                }
                "joint" => {
                    let name = ch
                        .attribute("name")
                        .map(|n| self.eval(n))
                        .unwrap_or_default();
                    let mut j = ControlJoint {
                        name,
                        command_interfaces: Vec::new(),
                        state_interfaces: Vec::new(),
                        loc: Loc::new(file, ch.range().start, ch.range().end),
                    };
                    for i in ch.descendants().filter(|x| x.is_element()) {
                        let iname = i.attribute("name").map(|n| self.eval(n));
                        match (i.tag_name().name(), iname) {
                            ("command_interface", Some(n)) => j.command_interfaces.push(n),
                            ("state_interface", Some(n)) => j.state_interfaces.push(n),
                            _ => {}
                        }
                    }
                    block.joints.push(j);
                }
                "sensor" => {
                    if let Some(n) = ch.attribute("name") {
                        let v = self.eval(n);
                        block.sensors.push(v);
                    }
                }
                "gpio" => {
                    if let Some(n) = ch.attribute("name") {
                        let v = self.eval(n);
                        block.gpios.push(v);
                    }
                }
                _ => {}
            }
        }
        block.joints.reverse();
        block
    }
}

struct Arith<'s, 'x, 'a, 'l> {
    s: &'s [u8],
    i: usize,
    x: &'x mut X<'a, 'l>,
}

impl Arith<'_, '_, '_, '_> {
    fn skip_ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        loop {
            self.skip_ws();
            match self.s.get(self.i) {
                Some(b'+') => {
                    self.i += 1;
                    v += self.term()?;
                }
                Some(b'-') => {
                    self.i += 1;
                    v -= self.term()?;
                }
                _ => return Some(v),
            }
        }
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.factor()?;
        loop {
            self.skip_ws();
            match self.s.get(self.i) {
                Some(b'*') => {
                    self.i += 1;
                    v *= self.factor()?;
                }
                Some(b'/') => {
                    self.i += 1;
                    v /= self.factor()?;
                }
                _ => return Some(v),
            }
        }
    }
    fn factor(&mut self) -> Option<f64> {
        self.skip_ws();
        match *self.s.get(self.i)? {
            b'(' => {
                self.i += 1;
                let v = self.expr()?;
                self.skip_ws();
                if self.s.get(self.i) == Some(&b')') {
                    self.i += 1;
                    Some(v)
                } else {
                    None
                }
            }
            b'-' => {
                self.i += 1;
                Some(-self.factor()?)
            }
            c if c.is_ascii_digit() || c == b'.' => {
                let st = self.i;
                while self.i < self.s.len()
                    && (self.s[self.i].is_ascii_digit()
                        || matches!(self.s[self.i], b'.' | b'e' | b'E'))
                {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[st..self.i]).ok()?.parse().ok()
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let st = self.i;
                while self.i < self.s.len()
                    && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_')
                {
                    self.i += 1;
                }
                let name = std::str::from_utf8(&self.s[st..self.i]).ok()?.to_string();
                if name == "pi" {
                    return Some(std::f64::consts::PI);
                }
                let v = self.x.lookup(&name)?.clone();
                let v = self.x.eval(&v);
                v.trim().parse().ok()
            }
            _ => None,
        }
    }
}

/// Builds a robot model from a URDF/xacro document. Returns None if the document is
/// not a robot description (e.g. a macro-only library produces an empty model).
pub fn extract<'a>(
    file: FileId,
    text: &'a str,
    loader: &dyn Loader<'a>,
) -> Result<Option<RobotModel>, String> {
    let doc = Document::parse(text).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if !root.has_tag_name("robot") {
        return Ok(None);
    }
    let mut x = X {
        loader,
        docs: vec![(file, Rc::new(doc))],
        scopes: vec![HashMap::new()],
        args: HashMap::new(),
        macros: HashMap::new(),
        blocks: Vec::new(),
        model: RobotModel {
            file,
            links: Vec::new(),
            joints: Vec::new(),
            control: Vec::new(),
            partial: false,
            inertials: Vec::new(),
            meshes: Vec::new(),
        },
        depth: 0,
        expanding: Vec::new(),
    };
    let rid = x.docs[0].1.root_element().id();
    x.process(0, rid, "robot");
    let m = x.model;
    if m.links.is_empty() && m.joints.is_empty() && m.control.is_empty() {
        return Ok(None);
    }
    Ok(Some(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoLoad;
    impl<'a> Loader<'a> for NoLoad {
        fn load(&self, _: FileId, _: &str) -> Option<(FileId, &'a str)> {
            None
        }
    }

    #[test]
    fn xacro_macros_and_control() {
        let src = r#"<?xml version="1.0"?>
<robot name="r" xmlns:xacro="http://www.ros.org/wiki/xacro">
  <xacro:property name="prefix" value="left_"/>
  <xacro:property name="w" value="0.2"/>
  <xacro:macro name="wheel" params="side *origin">
    <link name="${side}wheel"/>
    <joint name="${side}wheel_joint" type="continuous"><xacro:insert_block name="origin"/></joint>
  </xacro:macro>
  <link name="base_link"/>
  <xacro:wheel side="${prefix}"><origin xyz="0 ${w/2} 0"/></xacro:wheel>
  <xacro:wheel side="right_"><origin/></xacro:wheel>
  <ros2_control name="sys" type="system">
    <hardware><plugin>mock_components/GenericSystem</plugin></hardware>
    <joint name="left_wheel_joint"><command_interface name="velocity"/><state_interface name="position"/></joint>
  </ros2_control>
</robot>"#;
        let m = extract(0, src, &NoLoad).unwrap().unwrap();
        assert_eq!(m.links, vec!["base_link", "left_wheel", "right_wheel"]);
        assert_eq!(
            m.joints.iter().map(|j| j.name.as_str()).collect::<Vec<_>>(),
            vec!["left_wheel_joint", "right_wheel_joint"]
        );
        assert_eq!(
            m.control[0].hardware_plugin.as_deref(),
            Some("mock_components/GenericSystem")
        );
        assert_eq!(m.control[0].joints[0].command_interfaces, vec!["velocity"]);
        assert!(!m.partial);
    }

    /// A macro that forwards its own block to an inner macro (Kinova ros2_kortex
    /// `kortex_robot.xacro`). The forwarded `insert_block` must be evaluated in the caller's
    /// block scope; evaluating it in the inner macro's scope recursed until the stack overflowed.
    #[test]
    fn forwarded_block_is_evaluated_in_caller_scope() {
        let src = r#"<?xml version="1.0"?>
<robot name="r" xmlns:xacro="http://www.ros.org/wiki/xacro">
  <xacro:macro name="arm" params="prefix *origin">
    <link name="${prefix}base"/>
    <joint name="${prefix}base_joint" type="fixed"><xacro:insert_block name="origin"/></joint>
  </xacro:macro>
  <xacro:macro name="robot" params="prefix *origin">
    <xacro:arm prefix="${prefix}"><xacro:insert_block name="origin"/></xacro:arm>
  </xacro:macro>
  <link name="world"/>
  <xacro:robot prefix="k_"><origin xyz="0 0 0"/></xacro:robot>
</robot>"#;
        let m = extract(0, src, &NoLoad).unwrap().unwrap();
        assert_eq!(m.links, vec!["world", "k_base"]);
        assert_eq!(m.joints[0].name, "k_base_joint");
    }

    /// A block that inserts itself (invalid xacro) must not overflow the stack.
    #[test]
    fn self_inserting_block_terminates() {
        let src = r#"<robot xmlns:xacro="http://www.ros.org/wiki/xacro">
  <xacro:macro name="m" params="*b"><xacro:insert_block name="b"/></xacro:macro>
  <link name="x"/>
  <xacro:m><xacro:m><xacro:insert_block name="b"/></xacro:m></xacro:m>
</robot>"#;
        let m = extract(0, src, &NoLoad).unwrap().unwrap();
        assert_eq!(m.links, vec!["x"]);
    }

    /// Forwarding an unresolved parameter (`parent="${parent}"`, Kinova ros2_kortex) stores the
    /// literal `${parent}`; re-evaluating it on lookup must not recurse forever.
    #[test]
    fn self_referencing_value_terminates() {
        let src = r#"<robot xmlns:xacro="http://www.ros.org/wiki/xacro">
  <xacro:macro name="m" params="parent"><link name="${parent}_l"/></xacro:macro>
  <link name="x"/>
  <xacro:m parent="${parent}"/>
</robot>"#;
        let m = extract(0, src, &NoLoad).unwrap().unwrap();
        assert_eq!(m.links[0], "x");
        assert!(m.partial);
    }

    #[test]
    fn macro_library_is_not_a_model() {
        let src = r#"<robot xmlns:xacro="http://www.ros.org/wiki/xacro"><xacro:macro name="m" params=""><link name="x"/></xacro:macro></robot>"#;
        assert!(extract(0, src, &NoLoad).unwrap().is_none());
    }
}
