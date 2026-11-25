//! Chooses the assist model: the built-in ranker (default) or a downloadable tier.
//!
//! `--model` takes `builtin`, a tier name (`nano`, `mini`, `base`) or a path to a `.rfm`
//! file. A tier name only loads a file from the model directory whose SHA-256 is listed in
//! [`RELEASED`], so a swapped or damaged download is refused. An explicit path is the user's
//! own choice; its hash is shown in the model name. Any failure is reported and rosfix falls
//! back to the built-in ranker, so `--assist` never stops working.

use super::{learned::LearnedRanker, Query, Ranker};
use rosfix_model::{format, Model, Tier};
use std::path::{Path, PathBuf};

/// `(tier, sha256)` of every released tier file. Empty until the first tier passes the
/// evaluation gate and is published; until then tiers can only be tried by path.
pub const RELEASED: &[(&str, &str)] = &[];

/// The scoring head a tier must have for ranking.
pub const RANK_HEAD: &str = "rank";

/// Where tier files are installed: `%LOCALAPPDATA%\rosfix\models` on Windows, else
/// `$XDG_CACHE_HOME/rosfix/models` or `~/.cache/rosfix/models`.
pub fn model_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        if let Some(d) = std::env::var_os("LOCALAPPDATA") {
            return Some(PathBuf::from(d).join("rosfix").join("models"));
        }
    }
    if let Some(d) = std::env::var_os("XDG_CACHE_HOME").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d).join("rosfix").join("models"));
    }
    std::env::var_os("HOME").map(|h| {
        PathBuf::from(h)
            .join(".cache")
            .join("rosfix")
            .join("models")
    })
}

/// The model for `spec` (None = built-in).
pub fn load(spec: Option<&str>) -> Result<Box<dyn Ranker>, String> {
    match spec.unwrap_or("builtin") {
        "builtin" | "learned" => Ok(Box::new(LearnedRanker::new())),
        s => match Tier::parse(s) {
            Some(tier) => load_tier(tier),
            None => {
                let (m, sha) = format::load(Path::new(s))?;
                Ok(Box::new(TierRanker::new(m, &sha)?))
            }
        },
    }
}

fn load_tier(tier: Tier) -> Result<Box<dyn Ranker>, String> {
    let dir = model_dir().ok_or("no cache directory to look for models in")?;
    let path = dir.join(format!("{}.rfm", tier.name()));
    if !path.is_file() {
        return Err(format!(
            "no {} model installed (expected {}); no tier has been released yet, so pass a .rfm path to try one",
            tier.name(),
            path.display()
        ));
    }
    let (m, sha) = format::load(&path)?;
    if !RELEASED.iter().any(|(t, h)| *t == tier.name() && *h == sha) {
        return Err(format!(
            "{} is not a released rosfix {} model (sha256 {sha}); pass its path to --model to use it anyway",
            path.display(),
            tier.name()
        ));
    }
    Ok(Box::new(TierRanker::new(m, &sha)?))
}

/// The text a tier scores for one (finding, candidate) pair. Training must build its inputs
/// with this same function, exactly as the built-in ranker shares `learned::features`.
pub fn input_text(q: &Query, candidate: &str) -> String {
    format!(
        "[RULE] {} [CUR] {} [CAND] {candidate} [CTX] {}",
        q.rule, q.current, q.context
    )
}

/// A downloadable tier used as a ranker (cross-encoder: one forward pass per candidate).
pub struct TierRanker {
    model: Model,
    name: String,
}

impl TierRanker {
    pub fn new(model: Model, sha256: &str) -> Result<Self, String> {
        if model.head(RANK_HEAD).is_none() {
            return Err(format!("model `{}` has no `{RANK_HEAD}` head", model.tier));
        }
        let name = format!("{}@{}", model.tier, &sha256[..sha256.len().min(8)]);
        Ok(TierRanker { model, name })
    }
}

impl Ranker for TierRanker {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn score(&self, q: &Query) -> Option<Vec<f32>> {
        if q.candidates.is_empty() {
            return None;
        }
        q.candidates
            .iter()
            .map(|c| self.model.score(RANK_HEAD, &input_text(q, c)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rosfix_model::{net, Config, Tokenizer};

    fn tiny(heads: &[&str]) -> Model {
        let c = Config {
            vocab: 300,
            d: 16,
            layers: 1,
            heads: 2,
            ffn: 32,
            ctx: 64,
        };
        net::random("test", c, Tokenizer::new(Vec::new()), heads, 9)
    }

    fn query(c: &[String]) -> Query<'_> {
        Query {
            rule: "P003",
            current: "max_velocty",
            candidates: c,
            context: "max_velocty: 0.26",
        }
    }

    #[test]
    fn default_and_builtin_are_the_compiled_in_ranker() {
        assert_eq!(load(None).unwrap().name(), "learned");
        assert_eq!(load(Some("builtin")).unwrap().name(), "learned");
    }

    #[test]
    fn tier_names_need_a_released_file() {
        // Nothing is released yet: a tier name must fail (missing or unlisted file).
        assert!(RELEASED.is_empty());
        for t in ["nano", "mini", "base"] {
            assert!(load(Some(t)).is_err(), "{t} loaded without a release");
        }
    }

    #[test]
    fn explicit_file_scores_every_candidate() {
        let dir = std::env::temp_dir().join(format!("rosfix-registry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("good.rfm");
        let sha = format::save(&good, &tiny(&["rank"])).unwrap();
        let r = load(Some(good.to_str().unwrap())).unwrap();
        assert_eq!(r.name(), format!("test@{}", &sha[..8]));
        let c: Vec<String> = ["max_velocity", "min_velocity", "max_accel"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let s = r.score(&query(&c)).unwrap();
        assert_eq!(s.len(), 3);
        assert!(s.iter().all(|v| v.is_finite()));
        assert_eq!(s, r.score(&query(&c)).unwrap(), "scores are deterministic");
        assert!(r.score(&query(&[])).is_none());

        let no_head = dir.join("nohead.rfm");
        format::save(&no_head, &tiny(&["confidence"])).unwrap();
        assert!(load(Some(no_head.to_str().unwrap()))
            .err()
            .unwrap()
            .contains("rank"));
        assert!(load(Some(dir.join("missing.rfm").to_str().unwrap())).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
