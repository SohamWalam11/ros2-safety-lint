//! Lexical candidate ranking: edit distance + token overlap with ROS abbreviation
//! expansion. Deterministic, instant, and the fallback when no model is available.

use crate::util::edit_distance;
use std::collections::HashSet;

/// Common abbreviations in ROS parameter/topic/joint names → canonical word.
pub const ABBREV: &[(&str, &str)] = &[
    ("vel", "velocity"),
    ("velo", "velocity"),
    ("freq", "frequency"),
    ("hz", "frequency"),
    ("rate", "frequency"),
    ("tol", "tolerance"),
    ("ang", "angular"),
    ("lin", "linear"),
    ("acc", "acceleration"),
    ("accel", "acceleration"),
    ("decel", "deceleration"),
    ("dist", "distance"),
    ("rad", "radius"),
    ("thresh", "threshold"),
    ("pos", "position"),
    ("rot", "rotation"),
    ("cmd", "command"),
    ("ctrl", "controller"),
    ("ctl", "controller"),
    ("cfg", "config"),
    ("conf", "config"),
    ("param", "parameter"),
    ("params", "parameters"),
    ("msg", "message"),
    ("srv", "service"),
    ("sep", "separation"),
    ("diam", "diameter"),
    ("wheel_sep", "wheel_separation"),
    ("pub", "publish"),
    ("sub", "subscribe"),
    ("num", "number"),
    ("img", "image"),
    ("cam", "camera"),
    ("odom", "odometry"),
    ("tf", "transform"),
    ("lidar", "scan"),
    ("laser", "scan"),
    ("frame_id", "frame"),
    ("lft", "left"),
    ("rgt", "right"),
    ("l", "left"),
    ("r", "right"),
    ("fl", "front_left"),
    ("fr", "front_right"),
    ("rl", "rear_left"),
    ("rr", "rear_right"),
    ("min", "minimum"),
    ("max", "maximum"),
    ("timeout", "time"),
    ("dur", "duration"),
];

/// Splits `snake_case`, `camelCase`, `a/b`, `a.b` into lowercase canonical words.
pub fn tokens(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in s.chars() {
        if !ch.is_alphanumeric() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if ch.is_uppercase() && prev_lower && !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
        prev_lower = ch.is_lowercase() || ch.is_ascii_digit();
        cur.extend(ch.to_lowercase());
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
        .into_iter()
        .flat_map(|w| match ABBREV.iter().find(|(a, _)| *a == w) {
            Some((_, full)) => full.split('_').map(|x| x.to_string()).collect::<Vec<_>>(),
            None => vec![w],
        })
        .collect()
}

/// Similarity in [0, 1].
pub fn similarity(a: &str, b: &str) -> f32 {
    let (ta, tb): (HashSet<String>, HashSet<String>) = (
        tokens(a).into_iter().collect(),
        tokens(b).into_iter().collect(),
    );
    let inter = ta.intersection(&tb).count() as f32;
    let union = ta.union(&tb).count().max(1) as f32;
    let jaccard = inter / union;
    let (la, lb) = (a.to_lowercase(), b.to_lowercase());
    let max_len = la.chars().count().max(lb.chars().count()).max(1) as f32;
    let ed = 1.0 - edit_distance(&la, &lb) as f32 / max_len;
    let contain = if la.contains(&lb) || lb.contains(&la) {
        1.0
    } else {
        0.0
    };
    (0.5 * jaccard + 0.35 * ed + 0.15 * contain).clamp(0.0, 1.0)
}

/// Candidates sorted by similarity to `current` (best first). Ties keep input order.
pub fn rank(current: &str, candidates: &[String]) -> Vec<(String, f32)> {
    let mut v: Vec<(String, f32)> = candidates
        .iter()
        .map(|c| (c.clone(), similarity(current, c)))
        .collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abbreviations_bridge_large_edits() {
        let c: Vec<String> = [
            "max_linear_velocity",
            "min_linear_velocity",
            "controller_frequency",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let r = rank("max_lin_vel", &c);
        assert_eq!(r[0].0, "max_linear_velocity");
        assert!(r[0].1 - r[1].1 >= 0.15, "{r:?}");
        let r = rank("ctrl_freq", &c);
        assert_eq!(r[0].0, "controller_frequency");
    }

    #[test]
    fn tokenizer() {
        assert_eq!(tokens("leftWheelJoint"), vec!["left", "wheel", "joint"]);
        assert_eq!(tokens("base_link"), vec!["base", "link"]);
    }
}
