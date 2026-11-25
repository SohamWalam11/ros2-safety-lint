//! The built-in fix model: a tiny learned ranker in plain Rust (no ML runtime, no files).
//!
//! For each (wrong text, candidate) pair it computes [`N_FEATURES`] string features
//! (edit distances, n-gram overlap, abbreviation/prefix coverage, position of the candidate
//! in the lexical ranking, …) and scores them with a 16→16→1 MLP. Scores are compared across
//! the candidates of one finding (softmax), so the model learns *which* option is the
//! intended one rather than an absolute similarity.
//!
//! The weights (~290 floats, about 1 KB) are produced by `rosfix-train`
//! (crates/ros2-safety-mutate) and compiled into the binary from `learned_weights.rs`.
//! Training and inference share [`features`], so they cannot drift apart.

use super::lexical;
use super::Ranker;
use crate::util::edit_distance;
use std::collections::HashSet;

pub const N_FEATURES: usize = 16;
pub const N_HIDDEN: usize = 16;

/// Weights of the MLP: `w1[h][f]`, `b1[h]`, `w2[h]`, `b2`, plus feature normalisation.
#[derive(Debug, Clone, PartialEq)]
pub struct Weights {
    pub mean: [f32; N_FEATURES],
    pub std: [f32; N_FEATURES],
    pub w1: [[f32; N_FEATURES]; N_HIDDEN],
    pub b1: [f32; N_HIDDEN],
    pub w2: [f32; N_HIDDEN],
    pub b2: f32,
}

include!("learned_weights.rs");

/// The built-in weights.
pub fn builtin() -> &'static Weights {
    &BUILTIN
}

impl Weights {
    /// Normalised features → hidden activations (tanh) and the output score.
    #[allow(clippy::needless_range_loop)]
    pub fn forward(&self, x: &[f32; N_FEATURES]) -> ([f32; N_FEATURES], [f32; N_HIDDEN], f32) {
        let mut z = [0.0; N_FEATURES];
        for i in 0..N_FEATURES {
            z[i] = (x[i] - self.mean[i]) / self.std[i].max(1e-6);
        }
        let mut h = [0.0; N_HIDDEN];
        let mut out = self.b2;
        for j in 0..N_HIDDEN {
            let mut a = self.b1[j];
            for i in 0..N_FEATURES {
                a += self.w1[j][i] * z[i];
            }
            h[j] = a.tanh();
            out += self.w2[j] * h[j];
        }
        (z, h, out)
    }

    pub fn score_all(&self, current: &str, candidates: &[String]) -> Vec<f32> {
        features(current, candidates)
            .iter()
            .map(|x| self.forward(x).2)
            .collect()
    }
}

pub struct LearnedRanker(&'static Weights);

impl LearnedRanker {
    pub fn new() -> Self {
        LearnedRanker(builtin())
    }
}

impl Default for LearnedRanker {
    fn default() -> Self {
        Self::new()
    }
}

impl Ranker for LearnedRanker {
    fn name(&self) -> String {
        "learned".to_string()
    }

    /// The built-in model looks at the strings only; rule and context are ignored.
    fn score(&self, q: &super::Query) -> Option<Vec<f32>> {
        (!q.candidates.is_empty()).then(|| self.0.score_all(q.current, q.candidates))
    }
}

// ---------------------------------------------------------------------------- features

fn ngrams(s: &str, n: usize) -> HashSet<Vec<char>> {
    let c: Vec<char> = format!("^{s}$").chars().collect();
    if c.len() < n {
        return HashSet::new();
    }
    c.windows(n).map(|w| w.to_vec()).collect()
}

fn dice<T: std::hash::Hash + Eq>(a: &HashSet<T>, b: &HashSet<T>) -> f32 {
    let t = a.len() + b.len();
    if t == 0 {
        return 0.0;
    }
    2.0 * a.intersection(b).count() as f32 / t as f32
}

/// Optimal string alignment distance (edit distance that counts a transposition as one edit).
#[allow(clippy::needless_range_loop)]
fn osa(a: &[char], b: &[char]) -> usize {
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=m {
        d[0][j] = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    d[n][m]
}

/// Fraction of `a`'s characters that appear in order in `b` (abbreviations are subsequences).
fn subsequence_frac(a: &[char], b: &[char]) -> f32 {
    if a.is_empty() {
        return 0.0;
    }
    let mut j = 0;
    let mut hit = 0;
    for ch in a {
        while j < b.len() && b[j] != *ch {
            j += 1;
        }
        if j < b.len() {
            hit += 1;
            j += 1;
        }
    }
    hit as f32 / a.len() as f32
}

/// Fraction of words in `a` that are a prefix of some word in `b` (`lin` ⊂ `linear`).
fn prefix_cover(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() {
        return 0.0;
    }
    a.iter()
        .filter(|w| {
            b.iter()
                .any(|v| v.starts_with(w.as_str()) || w.starts_with(v.as_str()))
        })
        .count() as f32
        / a.len() as f32
}

fn raw_words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Features of every candidate for one finding (same order as `candidates`).
pub fn features(current: &str, candidates: &[String]) -> Vec<[f32; N_FEATURES]> {
    let q = current.to_lowercase();
    let qc: Vec<char> = q.chars().collect();
    let (q2, q3) = (ngrams(&q, 2), ngrams(&q, 3));
    let qw = raw_words(current);
    let qt: HashSet<String> = lexical::tokens(current).into_iter().collect();
    let qtv: Vec<String> = qt.iter().cloned().collect();
    let lex: Vec<f32> = candidates
        .iter()
        .map(|c| lexical::similarity(current, c))
        .collect();
    candidates
        .iter()
        .enumerate()
        .map(|(k, cand)| {
            let c = cand.to_lowercase();
            let cc: Vec<char> = c.chars().collect();
            let max_len = qc.len().max(cc.len()).max(1) as f32;
            let cw = raw_words(cand);
            let ct: HashSet<String> = lexical::tokens(cand).into_iter().collect();
            let ctv: Vec<String> = ct.iter().cloned().collect();
            let jacc = qt.intersection(&ct).count() as f32 / qt.union(&ct).count().max(1) as f32;
            let prefix = qc.iter().zip(&cc).take_while(|(a, b)| a == b).count() as f32 / max_len;
            let suffix = qc
                .iter()
                .rev()
                .zip(cc.iter().rev())
                .take_while(|(a, b)| a == b)
                .count() as f32
                / max_len;
            let better = lex.iter().filter(|v| **v > lex[k]).count();
            let best_other = lex
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != k)
                .map(|(_, v)| *v)
                .fold(0.0, f32::max);
            [
                lex[k],
                1.0 - edit_distance(&q, &c) as f32 / max_len,
                1.0 - osa(&qc, &cc) as f32 / max_len,
                jacc,
                dice(&q2, &ngrams(&c, 2)),
                dice(&q3, &ngrams(&c, 3)),
                prefix,
                suffix,
                subsequence_frac(&qc, &cc),
                subsequence_frac(&cc, &qc),
                prefix_cover(&qw, &cw),
                prefix_cover(&ctv, &qtv),
                qc.len().min(cc.len()) as f32 / max_len,
                (qw.len() as f32 - cw.len() as f32).abs() / qw.len().max(cw.len()).max(1) as f32,
                better as f32 / candidates.len().max(1) as f32,
                lex[k] - best_other,
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `assist::propose` would choose (None = abstain).
    fn pick(current: &str, cands: &[&str]) -> Option<String> {
        let c: Vec<String> = cands.iter().map(|s| s.to_string()).collect();
        let q = super::super::Query {
            rule: "P003",
            current,
            candidates: &c,
            context: "",
        };
        let s = LearnedRanker::new().score(&q).unwrap();
        super::super::decide(&s).map(|(i, _)| c[i].clone())
    }

    #[test]
    fn picks_intended_candidate() {
        let c = [
            "max_linear_velocity",
            "min_linear_velocity",
            "max_angular_velocity",
            "controller_frequency",
        ];
        assert_eq!(
            pick("max_lin_vel", &c).as_deref(),
            Some("max_linear_velocity")
        );
        assert_eq!(
            pick("max_angular_velocty", &c).as_deref(),
            Some("max_angular_velocity")
        );
        assert_eq!(
            pick("controler_frequency", &c).as_deref(),
            Some("controller_frequency")
        );
    }

    #[test]
    fn abstains_when_no_option_fits() {
        // A remap of a name the node never uses: neither option is meant.
        assert_eq!(pick("cmd_vel_typo", &["scan", "reset"]), None);
        assert_eq!(
            pick("use_sim_time", &["max_linear_velocity", "wheel_radius"]),
            None
        );
    }

    #[test]
    fn features_are_finite() {
        let c: Vec<String> = ["a", "", "x_y_z"].iter().map(|s| s.to_string()).collect();
        for f in features("", &c).iter().chain(features("ab", &c).iter()) {
            assert!(f.iter().all(|v| v.is_finite()), "{f:?}");
        }
    }
}
