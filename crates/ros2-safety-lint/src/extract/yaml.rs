//! A YAML tree that keeps exact byte spans, built on yaml-rust2's event parser.
//!
//! Scalar typing follows `rcl_yaml_param_parser` (the parser ROS 2 uses for
//! parameter files), not generic YAML 1.1, so that `1` vs `1.0` and `yes`/`on`
//! behave exactly as they do at runtime.

use crate::facts::ParamValue;
use std::collections::HashMap;
use yaml_rust2::parser::{MarkedEventReceiver, Parser};
use yaml_rust2::scanner::{Marker, TScalarStyle};
use yaml_rust2::Event;

#[derive(Debug, Clone)]
pub enum YNode {
    Scalar {
        value: String,
        quoted: bool,
        start: usize,
        end: usize,
    },
    Seq {
        items: Vec<YNode>,
        start: usize,
        end: usize,
    },
    Map {
        entries: Vec<(YNode, YNode)>,
        start: usize,
        end: usize,
    },
}

impl YNode {
    pub fn start(&self) -> usize {
        match self {
            YNode::Scalar { start, .. } | YNode::Seq { start, .. } | YNode::Map { start, .. } => {
                *start
            }
        }
    }

    pub fn end(&self) -> usize {
        match self {
            YNode::Scalar { end, .. } | YNode::Seq { end, .. } | YNode::Map { end, .. } => *end,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            YNode::Scalar { value, .. } => Some(value),
            _ => None,
        }
    }

    pub fn entries(&self) -> &[(YNode, YNode)] {
        match self {
            YNode::Map { entries, .. } => entries,
            _ => &[],
        }
    }

    pub fn items(&self) -> &[YNode] {
        match self {
            YNode::Seq { items, .. } => items,
            _ => &[],
        }
    }

    pub fn is_map(&self) -> bool {
        matches!(self, YNode::Map { .. })
    }

    /// Looks up a key in a mapping.
    pub fn get(&self, key: &str) -> Option<&YNode> {
        self.entry(key).map(|(_, v)| v)
    }

    pub fn entry(&self, key: &str) -> Option<(&YNode, &YNode)> {
        self.entries()
            .iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .map(|(k, v)| (k, v))
    }

    /// Converts to a ROS parameter value using rcl_yaml_param_parser semantics.
    pub fn to_param_value(&self) -> ParamValue {
        match self {
            YNode::Scalar { value, quoted, .. } => scalar_value(value, *quoted),
            YNode::Seq { items, .. } => {
                if items.is_empty() {
                    return ParamValue::EmptyArray;
                }
                let vals: Vec<ParamValue> = items.iter().map(|i| i.to_param_value()).collect();
                if let Some(v) = vals
                    .iter()
                    .map(|v| match v {
                        ParamValue::Bool(b) => Some(*b),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
                {
                    return ParamValue::BoolArray(v);
                }
                if let Some(v) = vals
                    .iter()
                    .map(|v| match v {
                        ParamValue::Int(i) => Some(*i),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
                {
                    return ParamValue::IntArray(v);
                }
                if let Some(v) = vals
                    .iter()
                    .map(|v| match v {
                        ParamValue::Double(d) => Some(*d),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
                {
                    return ParamValue::DoubleArray(v);
                }
                if let Some(v) = vals
                    .iter()
                    .map(|v| match v {
                        ParamValue::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
                {
                    return ParamValue::StrArray(v);
                }
                // Mixed element types are rejected by rcl; treat as unknown.
                ParamValue::Unknown
            }
            YNode::Map { .. } => ParamValue::Unknown,
        }
    }
}

const TRUE_WORDS: &[&str] = &[
    "Y", "y", "yes", "Yes", "YES", "true", "True", "TRUE", "on", "On", "ON",
];
const FALSE_WORDS: &[&str] = &[
    "N", "n", "no", "No", "NO", "false", "False", "FALSE", "off", "Off", "OFF",
];

/// rcl_yaml_param_parser scalar typing.
pub fn scalar_value(value: &str, quoted: bool) -> ParamValue {
    if quoted {
        return ParamValue::Str(value.to_string());
    }
    if TRUE_WORDS.contains(&value) {
        return ParamValue::Bool(true);
    }
    if FALSE_WORDS.contains(&value) {
        return ParamValue::Bool(false);
    }
    if let Some(i) = parse_int(value) {
        return ParamValue::Int(i);
    }
    if let Some(d) = parse_double(value) {
        return ParamValue::Double(d);
    }
    ParamValue::Str(value.to_string())
}

fn parse_int(s: &str) -> Option<i64> {
    let t = s.strip_prefix('+').unwrap_or(s);
    let digits = t.strip_prefix('-').unwrap_or(t);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse().ok()
}

fn parse_double(s: &str) -> Option<f64> {
    match s {
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => return Some(f64::INFINITY),
        "-.inf" | "-.Inf" | "-.INF" => return Some(f64::NEG_INFINITY),
        ".nan" | ".NaN" | ".NAN" => return Some(f64::NAN),
        _ => {}
    }
    let ok = s
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
        && s.bytes().any(|b| b.is_ascii_digit());
    if !ok {
        return None;
    }
    s.parse().ok()
}

// ---------------------------------------------------------------------------- builder

enum Frame {
    Seq {
        items: Vec<YNode>,
        start: usize,
        anchor: usize,
    },
    Map {
        entries: Vec<(YNode, YNode)>,
        pending_key: Option<YNode>,
        start: usize,
        anchor: usize,
    },
}

struct Builder<'a> {
    src: &'a str,
    /// char index → byte index (only built when the text is not pure ASCII).
    char_to_byte: Option<Vec<usize>>,
    stack: Vec<Frame>,
    docs: Vec<YNode>,
    anchors: HashMap<usize, YNode>,
}

impl<'a> Builder<'a> {
    fn byte(&self, m: &Marker) -> usize {
        match &self.char_to_byte {
            Some(t) => t.get(m.index()).copied().unwrap_or(self.src.len()),
            None => m.index().min(self.src.len()),
        }
    }

    fn scalar_end(&self, start: usize, value: &str, style: TScalarStyle) -> usize {
        let bytes = self.src.as_bytes();
        match style {
            TScalarStyle::SingleQuoted | TScalarStyle::DoubleQuoted => {
                let q = if style == TScalarStyle::SingleQuoted {
                    b'\''
                } else {
                    b'"'
                };
                let mut i = start + 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && q == b'"' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == q {
                        if q == b'\'' && i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                            i += 2;
                            continue;
                        }
                        return i + 1;
                    }
                    i += 1;
                }
                bytes.len()
            }
            TScalarStyle::Plain => (start + value.len()).min(bytes.len()),
            _ => {
                // Block scalars: extend to the end of the indented block (approximate: value length).
                (start + value.len()).min(bytes.len())
            }
        }
    }

    fn push_node(&mut self, node: YNode) {
        match self.stack.last_mut() {
            None => self.docs.push(node),
            Some(Frame::Seq { items, .. }) => items.push(node),
            Some(Frame::Map {
                entries,
                pending_key,
                ..
            }) => match pending_key.take() {
                None => *pending_key = Some(node),
                Some(k) => {
                    if k.as_str() == Some("<<") {
                        // Merge key: splice the referenced mapping(s) in.
                        let maps: Vec<YNode> = match node {
                            YNode::Seq { items, .. } => items,
                            other => vec![other],
                        };
                        for m in maps {
                            if let YNode::Map { entries: e, .. } = m {
                                for (mk, mv) in e {
                                    if !entries.iter().any(|(ek, _)| {
                                        ek.as_str().is_some() && ek.as_str() == mk.as_str()
                                    }) {
                                        entries.push((mk, mv));
                                    }
                                }
                            }
                        }
                    } else {
                        entries.push((k, node));
                    }
                }
            },
        }
    }
}

impl<'a> MarkedEventReceiver for Builder<'a> {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        let pos = self.byte(&mark);
        match ev {
            Event::Scalar(value, style, anchor, _tag) => {
                let end = self.scalar_end(pos, &value, style);
                let node = YNode::Scalar {
                    quoted: style != TScalarStyle::Plain,
                    value,
                    start: pos,
                    end,
                };
                if anchor > 0 {
                    self.anchors.insert(anchor, node.clone());
                }
                self.push_node(node);
            }
            Event::SequenceStart(anchor, _) => self.stack.push(Frame::Seq {
                items: Vec::new(),
                start: pos,
                anchor,
            }),
            Event::MappingStart(anchor, _) => self.stack.push(Frame::Map {
                entries: Vec::new(),
                pending_key: None,
                start: pos,
                anchor,
            }),
            Event::SequenceEnd => {
                if let Some(Frame::Seq {
                    items,
                    start,
                    anchor,
                }) = self.stack.pop()
                {
                    let node = YNode::Seq {
                        items,
                        start,
                        end: pos.max(start),
                    };
                    if anchor > 0 {
                        self.anchors.insert(anchor, node.clone());
                    }
                    self.push_node(node);
                }
            }
            Event::MappingEnd => {
                if let Some(Frame::Map {
                    entries,
                    start,
                    anchor,
                    ..
                }) = self.stack.pop()
                {
                    let node = YNode::Map {
                        entries,
                        start,
                        end: pos.max(start),
                    };
                    if anchor > 0 {
                        self.anchors.insert(anchor, node.clone());
                    }
                    self.push_node(node);
                }
            }
            Event::Alias(id) => {
                let node = self.anchors.get(&id).cloned().unwrap_or(YNode::Scalar {
                    value: String::new(),
                    quoted: false,
                    start: pos,
                    end: pos,
                });
                self.push_node(node);
            }
            _ => {}
        }
    }
}

/// Parses all documents in `src`. Returns the documents or a human-readable error.
pub fn parse(src: &str) -> Result<Vec<YNode>, String> {
    let char_to_byte = if src.is_ascii() {
        None
    } else {
        let mut t: Vec<usize> = src.char_indices().map(|(b, _)| b).collect();
        t.push(src.len());
        Some(t)
    };
    let mut b = Builder {
        src,
        char_to_byte,
        stack: Vec::new(),
        docs: Vec::new(),
        anchors: HashMap::new(),
    };
    let mut p = Parser::new_from_str(src);
    p.load(&mut b, true).map_err(|e| e.to_string())?;
    Ok(b.docs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_point_at_values() {
        let src = "node:\n  ros__parameters:\n    max_vel: 1\n    name: \"abc\"\n";
        let docs = parse(src).unwrap();
        let p = docs[0].get("node").unwrap().get("ros__parameters").unwrap();
        let (k, v) = p.entry("max_vel").unwrap();
        assert_eq!(&src[k.start()..k.end()], "max_vel");
        assert_eq!(&src[v.start()..v.end()], "1");
        let (_, v) = p.entry("name").unwrap();
        assert_eq!(&src[v.start()..v.end()], "\"abc\"");
    }

    #[test]
    fn rcl_typing() {
        assert_eq!(scalar_value("1", false), ParamValue::Int(1));
        assert_eq!(scalar_value("1.0", false), ParamValue::Double(1.0));
        assert_eq!(scalar_value("on", false), ParamValue::Bool(true));
        assert_eq!(scalar_value("1", true), ParamValue::Str("1".into()));
        assert_eq!(scalar_value("1e-3", false), ParamValue::Double(0.001));
        assert_eq!(scalar_value("abc", false), ParamValue::Str("abc".into()));
    }

    #[test]
    fn arrays_and_merge() {
        let src = "base: &b\n  a: 1\nx:\n  <<: *b\n  l: [1.0, 2.5]\n";
        let docs = parse(src).unwrap();
        let x = docs[0].get("x").unwrap();
        assert!(x.get("a").is_some());
        assert_eq!(
            x.get("l").unwrap().to_param_value(),
            ParamValue::DoubleArray(vec![1.0, 2.5])
        );
    }

    #[test]
    fn non_ascii_offsets() {
        let src = "# é comment\nk: v\n";
        let docs = parse(src).unwrap();
        let (k, _) = docs[0].entry("k").unwrap();
        assert_eq!(&src[k.start()..k.end()], "k");
    }
}
