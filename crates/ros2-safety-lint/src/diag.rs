//! Diagnostics and fixes produced by agents.

use crate::rules::{self, Rule, Severity};
use crate::source::{FileId, Loc};
use serde::Serialize;

/// A single text replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextEdit {
    pub file: FileId,
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

/// How confident we are that applying a fix is correct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FixSafety {
    /// Local, mechanical, verified by re-analysis: applied by `--fix --write`.
    Safe,
    /// Shown to the user but never applied automatically.
    Suggestion,
    /// Chosen by the fix assistant among valid candidates and verified by re-analysis;
    /// applied only with `--fix --write --accept-assist` (verification cannot prove intent).
    Assisted,
}

/// Where a finding can be repaired by replacing one span with one of a closed set of
/// valid values taken from the workspace facts (declared parameters, joints, frames…).
#[derive(Debug, Clone, Serialize)]
pub struct RepairSite {
    /// The exact text to replace.
    pub span: Loc,
    /// The current (wrong) text.
    pub current: String,
    /// Valid replacement values; the assistant may only pick one of these.
    pub candidates: Vec<String>,
    /// Wrap the chosen value in quotes when writing it.
    pub quote: Option<char>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Fix {
    pub description: String,
    pub safety: FixSafety,
    pub edits: Vec<TextEdit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Related {
    pub loc: Loc,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub rule: &'static str,
    pub severity: Severity,
    pub message: String,
    pub loc: Loc,
    pub related: Vec<Related>,
    pub help: Option<String>,
    pub fix: Option<Fix>,
    pub repair: Option<RepairSite>,
    /// Background references found by `--online` lookups (title, URL).
    pub references: Vec<(String, String)>,
}

impl Diagnostic {
    /// Creates a diagnostic with the rule's default severity.
    pub fn new(rule_id: &'static str, loc: Loc, message: impl Into<String>) -> Self {
        let rule = rules::find(rule_id).unwrap_or_else(|| panic!("unknown rule {rule_id}"));
        Diagnostic {
            rule: rule.id,
            severity: rule.severity,
            message: message.into(),
            loc,
            related: Vec::new(),
            help: None,
            fix: None,
            repair: None,
            references: Vec::new(),
        }
    }

    /// Declares a repair site: `span` may be replaced by one of `candidates`.
    /// Ignored when the span is empty or there are no candidates.
    pub fn repair(
        mut self,
        span: Loc,
        current: impl Into<String>,
        candidates: Vec<String>,
    ) -> Self {
        let current = current.into();
        let mut c: Vec<String> = candidates
            .into_iter()
            .filter(|x| *x != current && !x.is_empty())
            .collect();
        c.sort();
        c.dedup();
        if span.end > span.start && !c.is_empty() {
            self.repair = Some(RepairSite {
                span,
                current,
                candidates: c,
                quote: None,
            });
        }
        self
    }

    pub fn related(mut self, loc: Loc, message: impl Into<String>) -> Self {
        self.related.push(Related {
            loc,
            message: message.into(),
        });
        self
    }

    pub fn help(mut self, h: impl Into<String>) -> Self {
        self.help = Some(h.into());
        self
    }

    pub fn severity(mut self, s: Severity) -> Self {
        self.severity = s;
        self
    }

    pub fn fix(mut self, f: Fix) -> Self {
        self.fix = Some(f);
        self
    }

    /// Convenience: a single-edit safe fix.
    pub fn safe_fix(
        self,
        description: impl Into<String>,
        file: FileId,
        start: usize,
        end: usize,
        replacement: impl Into<String>,
    ) -> Self {
        self.fix(Fix {
            description: description.into(),
            safety: FixSafety::Safe,
            edits: vec![TextEdit {
                file,
                start,
                end,
                replacement: replacement.into(),
            }],
        })
    }

    pub fn rule_info(&self) -> &'static Rule {
        rules::find(self.rule).expect("rule registered")
    }
}
