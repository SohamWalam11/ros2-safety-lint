//! Constraints inferred from the guards a node writes around its own parameters (the idea of
//! SPEX, Xu et al., SOSP 2013, applied to ROS 2 parameters):
//!
//! ```cpp
//! rate_ = declare_parameter("rate", 10.0);
//! if (rate_ <= 0.0) {
//!   RCLCPP_FATAL(get_logger(), "rate must be positive");
//!   throw std::invalid_argument("rate");
//! }
//! ```
//!
//! The node throws when `rate <= 0`, so it requires `rate > 0`. Only a narrow, certain form is
//! accepted: a single comparison of a variable against a numeric literal whose body throws
//! (optionally after one logging statement), where the variable was set from exactly one
//! parameter earlier in the same file, with no parameter-callback code in between (a guard
//! inside an `on_set_parameters` callback rejects later changes, not the startup value).

use crate::facts::Constraint;
use std::collections::HashMap;

fn re(pattern: &str) -> regex::Regex {
    regex::Regex::new(pattern).expect("valid regex")
}

/// (parameter name, constraint the node enforces by throwing).
pub fn guard_constraints(src: &str) -> Vec<(String, Constraint)> {
    if !src.contains("throw") {
        return Vec::new();
    }
    // variable → (parameter, byte offset of the binding); a variable bound to two different
    // parameters is ambiguous and dropped.
    let mut bound: HashMap<String, Option<(String, usize)>> = HashMap::new();
    let mut bind = |var: &str, param: &str, at: usize| {
        let e = bound.entry(var.to_string()).or_insert_with(|| Some((param.to_string(), at)));
        if let Some((p, _)) = e {
            if p != param {
                *e = None;
            }
        }
    };
    let assign = re(r#"(\w+)\s*=\s*(?:this\s*->\s*)?(?:\w+\s*->\s*)?(?:declare_parameter|get_parameter_or)\s*(?:<[^;()]*>)?\s*\(\s*"([^"\s]+)""#);
    for c in assign.captures_iter(src) {
        bind(&c[1], &c[2], c.get(0).map_or(0, |m| m.start()));
    }
    let get_into = re(r#"get_parameter\s*\(\s*"([^"\s]+)"\s*,\s*(\w+)\s*\)"#);
    for c in get_into.captures_iter(src) {
        bind(&c[2], &c[1], c.get(0).map_or(0, |m| m.start()));
    }
    let get_as = re(r#"(\w+)\s*=\s*(?:this\s*->\s*)?(?:\w+\s*->\s*)?get_parameter\s*\(\s*"([^"\s]+)"\s*\)\s*\.\s*(?:as_double|as_int|get_value\s*<[^>]*>)\s*\(\s*\)"#);
    for c in get_as.captures_iter(src) {
        bind(&c[1], &c[2], c.get(0).map_or(0, |m| m.start()));
    }

    let guard = re(r"if\s*\(\s*(\w+)\s*(<=|<|>=|>)\s*(-?\d+(?:\.\d*)?(?:[eE][-+]?\d+)?)[fFlLuU]?\s*\)\s*\{?\s*(?:RCLCPP_(?:ERROR|FATAL)\w*\s*\([^;]*\)\s*;\s*)?throw\b");
    let mut out: Vec<(String, Constraint)> = Vec::new();
    for c in guard.captures_iter(src) {
        let Some(Some((param, at))) = bound.get(&c[1]) else { continue };
        let pos = c.get(0).map_or(0, |m| m.start());
        if pos < *at || src[*at..pos].contains("SetParametersResult") {
            continue;
        }
        let Ok(v) = c[3].parse::<f64>() else { continue };
        // The node throws when the condition holds: it requires the opposite.
        let k = match &c[2] {
            "<=" => Constraint::Lower { bound: v, inclusive: false },
            "<" => Constraint::Lower { bound: v, inclusive: true },
            ">=" => Constraint::Upper { bound: v, inclusive: false },
            ">" => Constraint::Upper { bound: v, inclusive: true },
            _ => continue,
        };
        if !out.iter().any(|(p, x)| p == param && *x == k) {
            out.push((param.clone(), k));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throwing_guards_become_constraints() {
        let src = r#"
class N : public rclcpp::Node { public: N() : Node("n") {
  rate_ = declare_parameter("rate", 10.0);
  if (rate_ <= 0.0) {
    RCLCPP_FATAL(get_logger(), "rate must be positive");
    throw std::invalid_argument("rate");
  }
  int depth;
  get_parameter("depth", depth);
  if (depth > 100) throw std::runtime_error("too deep");
  max_ = get_parameter("max").as_double();
  if (max_ < 1.5) { RCLCPP_WARN(get_logger(), "low"); }
  auto cb = [this](const std::vector<rclcpp::Parameter> & p) { rcl_interfaces::msg::SetParametersResult r;
    if (gain_ < 0) throw std::runtime_error("x"); return r; };
  gain_ = declare_parameter("gain", 1.0);
} };
"#;
        let mut c = guard_constraints(src);
        c.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            c,
            vec![
                ("depth".to_string(), Constraint::Upper { bound: 100.0, inclusive: true }),
                ("rate".to_string(), Constraint::Lower { bound: 0.0, inclusive: false }),
            ],
            "only throwing guards after the binding; a warning or a callback guard says nothing"
        );
    }
}
