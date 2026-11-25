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
pub mod registry;

use crate::diag::{Fix, FixSafety, TextEdit};
use crate::engine::Analysis;

/// What a ranker sees for one finding: the closed candidate set plus its context.
#[derive(Debug, Clone, Copy)]
pub struct Query<'a> {
    /// Rule id, e.g. `P003`.
    pub rule: &'a str,
    /// The wrong text at the repair site.
    pub current: &'a str,
    /// The top lexical candidates (at most [`TOP_K`]); a ranker picks among these only.
    pub candidates: &'a [String],
    /// A few source lines around the repair site (empty when unavailable).
    pub context: &'a str,
}

/// Something that can score candidates for the wrong text of a finding (higher is better).
pub trait Ranker {
    /// Shown in fix descriptions, e.g. `learned` or `mini@1a2b3c4d`.
    fn name(&self) -> String;
    /// One score per candidate (compared through a softmax against "none of these"), or
    /// None on failure.
    fn score(&self, q: &Query) -> Option<Vec<f32>>;
}

/// How many lexical candidates are passed on to the ranker.
pub const TOP_K: usize = 8;

#[derive(Debug, Clone)]
pub struct Proposal {
    /// Index into `Analysis::diagnostics`.
    pub diag: usize,
    pub value: String,
    pub by: String,
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
                let context = context_lines(a, r.span.file, r.span.start, CONTEXT_LINES);
                let q = Query {
                    rule: d.rule,
                    current: &r.current,
                    candidates: &top,
                    context: &context,
                };
                match m.score(&q) {
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
            by: by.clone(),
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

/// Lines of source on each side of the repair site given to rankers.
pub const CONTEXT_LINES: usize = 3;

/// The repair site's line with `n` lines before and after (at most 600 bytes).
pub fn context_lines(a: &Analysis, file: crate::source::FileId, byte: usize, n: usize) -> String {
    let f = a.index.file(file);
    let (line, _) = f.line_col(byte.min(f.text.len()));
    let lines: Vec<&str> = f.text.lines().collect();
    let lo = line.saturating_sub(1 + n);
    let hi = (line + n).min(lines.len());
    let mut s = lines.get(lo..hi).unwrap_or_default().join("\n");
    if s.len() > 600 {
        let mut cut = 600;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
    }
    s
}

fn lexical_choice(ranked: &[(String, f32)]) -> Option<(String, String, f32)> {
    let (best, s1) = ranked.first()?;
    let s2 = ranked.get(1).map_or(0.0, |x| x.1);
    (*s1 >= 0.55 && s1 - s2 >= 0.15).then(|| (best.clone(), "lexical".to_string(), *s1))
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
