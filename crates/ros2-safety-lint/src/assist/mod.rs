//! The fix assistant: chooses a repair for findings that have no deterministic fix.
//!
//! Ladder (each step only runs when the previous one produced nothing):
//! 1. deterministic templates (in the agents),
//! 2. lexical ranking of the finding's closed candidate set (always available),
//! 3. the built-in learned ranker re-scoring those candidates (`learned`, ~1 KB of
//!    weights compiled into the binary; no ML runtime, no model files),
//! 4. explanation only.
//!
//! The assistant never writes free text: it can only pick one of the candidates an
//! agent attached to the finding (declared parameters, joints, frames, executables…).
//! Every pick is then verified by re-analysis in the fix engine, and assisted fixes are
//! written only with `--accept-assist`.

pub mod learned;
pub mod lexical;
pub mod online;

use crate::diag::{Fix, FixSafety, TextEdit};
use crate::engine::Analysis;

/// Something that can score candidates for the wrong text of a finding (higher is better).
pub trait Ranker {
    fn name(&self) -> &'static str;
    /// One score per candidate (compared through a softmax), or None on failure.
    fn score(&self, current: &str, candidates: &[String]) -> Option<Vec<f32>>;
}

/// How many lexical candidates are passed on to the ranker.
pub const TOP_K: usize = 8;

#[derive(Debug, Clone)]
pub struct Proposal {
    /// Index into `Analysis::diagnostics`.
    pub diag: usize,
    pub value: String,
    pub by: &'static str,
    /// Confidence in [0, 1].
    pub confidence: f32,
    pub fix: Fix,
}

/// Proposes assisted fixes for findings that carry a repair site and no safe fix.
pub fn propose(a: &Analysis, model: Option<&dyn Ranker>) -> Vec<Proposal> {
    let mut out = Vec::new();
    for (i, d) in a.diagnostics.iter().enumerate() {
        let Some(r) = &d.repair else { continue };
        if d.fix.as_ref().is_some_and(|f| f.safety == FixSafety::Safe) {
            continue;
        }
        let ranked = lexical::rank(&r.current, &r.candidates);
        if ranked.is_empty() {
            continue;
        }
        let choice = match model {
            Some(m) => {
                let top: Vec<String> = ranked.iter().take(TOP_K).map(|(c, _)| c.clone()).collect();
                match m.score(&r.current, &top) {
                    Some(s) => decide(&s).map(|(best, pb)| (top[best].clone(), m.name(), pb)),
                    None => lexical_choice(&ranked),
                }
            }
            None => lexical_choice(&ranked),
        };
        let Some((value, by, conf)) = choice else {
            continue;
        };
        let replacement = match r.quote {
            Some(q) => format!("{q}{value}{q}"),
            None => value.clone(),
        };
        out.push(Proposal {
            diag: i,
            value: value.clone(),
            by,
            confidence: conf,
            fix: Fix {
                description: format!(
                    "Replace `{}` with `{value}` (assistant: {by}, confidence {:.0}%)",
                    r.current,
                    conf * 100.0
                ),
                safety: FixSafety::Assisted,
                edits: vec![TextEdit {
                    file: r.span.file,
                    start: r.span.start,
                    end: r.span.end,
                    replacement,
                }],
            },
        });
    }
    out
}

fn lexical_choice(ranked: &[(String, f32)]) -> Option<(String, &'static str, f32)> {
    let (best, s1) = ranked.first()?;
    let s2 = ranked.get(1).map_or(0.0, |x| x.1);
    (*s1 >= 0.55 && s1 - s2 >= 0.15).then(|| (best.clone(), "lexical", *s1))
}

pub fn softmax(x: &[f32]) -> Vec<f32> {
    let m = x.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let e: Vec<f32> = x.iter().map(|v| (v - m).exp()).collect();
    let s: f32 = e.iter().sum();
    e.iter().map(|v| v / s.max(f32::MIN_POSITIVE)).collect()
}

fn argmax(x: &[f32]) -> (usize, f32) {
    x.iter()
        .enumerate()
        .fold((0, f32::NEG_INFINITY), |acc, (i, v)| {
            if *v > acc.1 {
                (i, *v)
            } else {
                acc
            }
        })
}

/// Attaches proposals to their diagnostics (as fixes, unless a fix already exists).
pub fn attach(a: &mut Analysis, proposals: Vec<Proposal>) {
    for p in proposals {
        let d = &mut a.diagnostics[p.diag];
        if d.fix.is_none()
            || d.fix
                .as_ref()
                .is_some_and(|f| f.safety == FixSafety::Suggestion)
        {
            d.fix = Some(p.fix);
        }
    }
}

/// The built-in ranker (always available: its weights are compiled in).
pub fn load_model() -> Box<dyn Ranker> {
    Box::new(learned::LearnedRanker::new())
}

/// Score of the implicit "none of these options" choice. Ranker scores are learned relative
/// to it, so the ranker can abstain when no option is right (e.g. a remap of a name the node
/// does not use at all) instead of picking the least bad one.
pub const NONE_SCORE: f32 = 0.0;

/// Picks among candidates the way [`propose`] does with a ranker: (index, probability), or
/// None when "none of these" wins or there is no clear winner. Shared with `rosfix-train`.
pub fn decide(scores: &[f32]) -> Option<(usize, f32)> {
    let mut with_none = scores.to_vec();
    with_none.push(NONE_SCORE);
    let probs = softmax(&with_none);
    let (best, pb) = argmax(&probs);
    if best == scores.len() {
        return None;
    }
    let second = probs
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != best)
        .map(|(_, p)| *p)
        .fold(0.0, f32::max);
    (pb >= 0.5 && pb - second >= 0.2).then_some((best, pb))
}

/// The lexical decision on its own (index into `candidates`), for baselines.
pub fn decide_lexical(current: &str, candidates: &[String]) -> Option<usize> {
    let ranked = lexical::rank(current, candidates);
    let (v, _, _) = lexical_choice(&ranked)?;
    candidates.iter().position(|c| *c == v)
}
