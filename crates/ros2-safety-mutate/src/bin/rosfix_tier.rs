//! rosfix-tier: data checks and training for the downloadable assist tiers.
//!
//! `rosfix-tier train` trains a tier on the `--dump-tier` cases with the protocol of
//! `rosfix-train`: per fold, train on three folds, calibrate the abstention offset on the
//! name-disjoint cases of the next fold, test on the name-disjoint cases of the fold. It scores
//! with the exported int8 model (what would ship) and compares, case by case, with the built-in
//! ranker's calibrated fold model (`rosfix-train --decisions`).
//!
//! `rosfix-tier stats CASES.jsonl MERGES.txt` reads the cases written by
//! `rosfix-train --dump-tier` and the tokenizer from `rosfix-model-bpe`, and reports per rule
//! how many cases there are, how long the inputs are in bytes and tokens, how many would be
//! truncated at each tier's context length, and whether any source context contains the
//! intended answer (it must not: the answer is the name the user failed to write; cases whose
//! written name is itself the target, `absent: unrelated name`, are not counted).

use rosfix::assist::registry::RANK_HEAD;
use rosfix::assist::{decide, decide_lexical};
use rosfix_model::train::{Adam, Example, Rng, TrainModel, SHARDS};
use rosfix_model::{Model, Tier, Tokenizer};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub struct TierCase {
    pub fold: u8,
    pub op: String,
    pub target: String,
    pub current: String,
    pub rule: String,
    pub options: Vec<String>,
    pub answer: Option<usize>,
    pub inputs: Vec<String>,
}

pub fn read_cases(path: &str) -> Vec<TierCase> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: serde_json::Value =
                serde_json::from_str(l).unwrap_or_else(|e| panic!("{path}: {e}"));
            let strs = |k: &str| -> Vec<String> {
                v[k].as_array()
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default()
            };
            TierCase {
                fold: v["fold"].as_u64().unwrap_or(0) as u8,
                op: v["op"].as_str().unwrap_or("").to_string(),
                target: v["target"].as_str().unwrap_or("").to_string(),
                current: v["current"].as_str().unwrap_or("").to_string(),
                rule: v["rule"].as_str().unwrap_or("").to_string(),
                options: strs("options"),
                answer: v["answer"].as_u64().map(|a| a as usize),
                inputs: strs("inputs"),
            }
        })
        .collect()
}

fn pct(v: &mut [usize], p: f64) -> usize {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn stats(cases: &[TierCase], tok: &Tokenizer) {
    let tiers = [Tier::Nano, Tier::Mini, Tier::Base];
    let mut by_rule: BTreeMap<&str, Vec<&TierCase>> = BTreeMap::new();
    for c in cases {
        by_rule.entry(c.rule.as_str()).or_default().push(c);
        by_rule.entry("ALL").or_default().push(c);
    }
    let ctx_names: Vec<String> = tiers
        .iter()
        .map(|t| format!("cut@{} ({})", t.config().ctx, t.name()))
        .collect();
    println!(
        "| rule | cases | answer offered | inputs | bytes mean / p95 | tokens mean / p95 / max | {} | context has answer |",
        ctx_names.join(" | ")
    );
    println!("|---|---:|---:|---:|---|---|{}---:|", "---:|".repeat(tiers.len()));
    for (rule, cs) in &by_rule {
        let mut bytes = Vec::new();
        let mut toks = Vec::new();
        let mut leaks = 0usize;
        for c in cs {
            for inp in &c.inputs {
                bytes.push(inp.len());
                toks.push(tok.encode(inp, usize::MAX).len());
            }
            if let Some(ctx) = c.inputs.first().and_then(|s| s.split_once(" [CTX] ")) {
                let words = ctx.1.split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '.'));
                if c.current != c.target && words.into_iter().any(|w| w == c.target) {
                    leaks += 1;
                }
            }
        }
        let n_in = toks.len().max(1);
        let mean = |v: &[usize]| v.iter().sum::<usize>() as f64 / v.len().max(1) as f64;
        let cut: Vec<String> = tiers
            .iter()
            .map(|t| {
                let c = toks.iter().filter(|&&n| n > t.config().ctx as usize).count();
                format!("{:.1}%", 100.0 * c as f64 / n_in as f64)
            })
            .collect();
        let offered = cs.iter().filter(|c| c.answer.is_some()).count();
        let (bm, tm, tmax) = (mean(&bytes), mean(&toks), toks.iter().copied().max().unwrap_or(0));
        println!(
            "| {rule} | {} | {:.1}% | {} | {bm:.0} / {} | {tm:.1} / {} / {tmax} | {} | {leaks} |",
            cs.len(),
            100.0 * offered as f64 / cs.len() as f64,
            toks.len(),
            pct(&mut bytes, 0.95),
            pct(&mut toks, 0.95),
            cut.join(" | ")
        );
    }
    let mut per_fold = [0usize; 5];
    for c in cases {
        per_fold[c.fold as usize % 5] += 1;
    }
    let mut ops: BTreeMap<&str, usize> = BTreeMap::new();
    for c in cases {
        *ops.entry(c.op.as_str()).or_default() += 1;
    }
    let bad = cases
        .iter()
        .filter(|c| c.inputs.len() != c.options.len() || c.answer.is_some_and(|a| a >= c.options.len()))
        .count();
    println!("\ncases per fold: {per_fold:?}; malformed cases: {bad}");
    println!("cases per corruption: {ops:?}");
}

// ---------------------------------------------------------------------------- training

/// 0 correct, 1 abstained, 2 wrong edit: the same judgement as `rosfix-train`.
fn judge(pick: Option<usize>, answer: Option<usize>) -> usize {
    match (pick, answer) {
        (Some(p), Some(a)) if p == a => 0,
        (None, _) => 1,
        _ => 2,
    }
}

/// Scores of every case's options under the int8 runtime model, on [`SHARDS`] threads.
fn score_all(model: &Model, cases: &[&TierCase]) -> Vec<Vec<f32>> {
    let part = cases.len().div_ceil(SHARDS).max(1);
    std::thread::scope(|s| {
        let hs: Vec<_> = cases
            .chunks(part)
            .map(|chunk| {
                s.spawn(move || {
                    chunk
                        .iter()
                        .map(|c| {
                            c.inputs
                                .iter()
                                .map(|t| model.score(RANK_HEAD, t).expect("rank head"))
                                .collect()
                        })
                        .collect::<Vec<Vec<f32>>>()
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().expect("scoring thread")).collect()
    })
}

fn pick(scores: &[f32], shift: f32) -> Option<usize> {
    let s: Vec<f32> = scores.iter().map(|x| x - shift).collect();
    decide(&s).map(|x| x.0)
}

struct Opts {
    tier: Tier,
    epochs: usize,
    batch: usize,
    lr: f32,
    seed: u64,
    max_steps: usize,
}

/// Trains on all folds but `test` and `test + 1`, calibrates on the name-disjoint cases of
/// `test + 1`, evaluates on the name-disjoint cases of `test`, exactly like `rosfix-train`.
fn run_fold(
    cases: &[TierCase],
    builtin: &BTreeMap<usize, Decision>,
    base: Option<&BaseScores>,
    tok: &Tokenizer,
    test: u8,
    o: &Opts,
    out: &Path,
) -> serde_json::Value {
    let calib = (test + 1) % FOLDS;
    let train: Vec<(usize, &TierCase)> = cases
        .iter()
        .enumerate()
        .filter(|(_, c)| c.fold != test && c.fold != calib)
        .collect();
    let seen: BTreeSet<&str> = train.iter().map(|c| c.1.target.as_str()).collect();
    // Residual mode: the built-in ranker's fold-`test` model scores, added to the tier's.
    let base_of = |idx: usize| -> Vec<f32> {
        base.map(|b| b.get(&(test, idx)).cloned().unwrap_or_else(|| panic!("no base scores for fold {test} case {idx}")))
            .unwrap_or_default()
    };
    let unseen = |f: u8| -> Vec<(usize, &TierCase)> {
        cases
            .iter()
            .enumerate()
            .filter(|(_, c)| c.fold == f && !seen.contains(c.target.as_str()))
            .collect()
    };
    let calib_set = unseen(calib);
    let test_set = unseen(test);
    let cfg = o.tier.config();
    let ctx = cfg.ctx as usize;
    let data: Vec<Example> = train
        .iter()
        .map(|(idx, c)| Example {
            inputs: c.inputs.iter().map(|t| tok.encode(t, ctx)).collect(),
            base: base_of(*idx),
            answer: c.answer,
        })
        .collect();
    let mut m = TrainModel::init(o.tier.name(), cfg, &[RANK_HEAD], o.seed ^ u64::from(test));
    let head = m.head_index(RANK_HEAD).expect("rank head");
    if base.is_some() {
        // Start exactly at the built-in ranker: the residual is 0 until training moves it.
        m.heads[head].w.w.iter_mut().for_each(|w| *w = 0.0);
    }
    let steps_per_epoch = data.len().div_ceil(o.batch);
    let total = (steps_per_epoch * o.epochs).min(o.max_steps);
    let warmup = (total / 20).max(1);
    eprintln!(
        "fold {test}: {} train / {} calibration / {} test cases; {total} steps of {}",
        data.len(),
        calib_set.len(),
        test_set.len(),
        o.batch
    );
    let t0 = Instant::now();
    let mut rng = Rng(o.seed.wrapping_mul(31) ^ u64::from(test) ^ 0x5eed);
    let mut order: Vec<usize> = (0..data.len()).collect();
    let mut step = 0usize;
    let mut log = Vec::new();
    'outer: for epoch in 0..o.epochs {
        for i in (1..order.len()).rev() {
            order.swap(i, rng.below(i + 1));
        }
        let (mut sum, mut n) = (0.0f64, 0usize);
        for chunk in order.chunks(o.batch) {
            if step >= total {
                break 'outer;
            }
            // Linear warm-up, then linear decay to a tenth.
            let lr = if step < warmup {
                o.lr * (step + 1) as f32 / warmup as f32
            } else {
                let t = (step - warmup) as f32 / (total - warmup).max(1) as f32;
                o.lr * (1.0 - 0.9 * t)
            };
            let opt = Adam {
                lr,
                ..Adam::default()
            };
            let batch: Vec<Example> = chunk.iter().map(|&i| data[i].clone()).collect();
            sum += f64::from(m.train_step(head, &batch, &opt));
            n += batch.len();
            step += 1;
            if step.is_multiple_of(50) || step == total {
                let s = t0.elapsed().as_secs_f64();
                eprintln!(
                    "  fold {test} epoch {epoch} step {step}/{total}: loss {:.4}, {:.2} s/step, {:.0} s",
                    sum / n.max(1) as f64,
                    s / step as f64,
                    s
                );
            }
        }
        log.push(serde_json::json!({"epoch": epoch, "loss": sum / n.max(1) as f64}));
    }
    let train_secs = t0.elapsed().as_secs_f64();
    let meta = serde_json::json!({
        "tier": o.tier.name(), "fold": test, "seed": o.seed, "steps": step, "batch": o.batch,
        "lr": o.lr, "train_cases": data.len(), "residual_on": if base.is_some() { "builtin" } else { "" },
    });
    let mut model = m.export(tok.clone(), &meta.to_string());

    // Calibration: lower all scores until no more wrong edits than the lexical ranker.
    let cal_cases: Vec<&TierCase> = calib_set.iter().map(|x| x.1).collect();
    let cal_scores = add_base(score_all(&model, &cal_cases), calib_set.iter().map(|x| base_of(x.0)));
    let lex_wrong = cal_cases
        .iter()
        .filter(|c| judge(decide_lexical(&c.current, &c.options), c.answer) == 2)
        .count();
    let mut shift = 0.0f32;
    for k in 0..=40 {
        shift = k as f32 * 0.1;
        let wrong = cal_cases
            .iter()
            .zip(&cal_scores)
            .filter(|(c, s)| judge(pick(s, shift), c.answer) == 2)
            .count();
        if wrong <= lex_wrong {
            break;
        }
    }
    let h = model
        .heads
        .iter_mut()
        .find(|h| h.name == RANK_HEAD)
        .expect("rank head");
    h.b -= shift;

    // Held-out test, case by case against the built-in ranker's fold model.
    let test_cases: Vec<&TierCase> = test_set.iter().map(|x| x.1).collect();
    let scores = add_base(score_all(&model, &test_cases), test_set.iter().map(|x| base_of(x.0)));
    let (mut tier, mut bi, mut lex) = ([0usize; 3], [0usize; 3], [0usize; 3]);
    let (mut tier_only, mut builtin_only) = (0usize, 0usize);
    for ((idx, c), s) in test_set.iter().zip(&scores) {
        let d = builtin.get(idx).unwrap_or_else(|| {
            panic!("no built-in decision for case {idx}: run rosfix-train --decisions with the arguments used for --dump-tier")
        });
        assert!(d.fold == test && d.target == c.target && d.current == c.current, "decisions file does not match the cases (case {idx})");
        let t = judge(pick(s, 0.0), c.answer);
        let b = judge(d.builtin, c.answer);
        tier[t] += 1;
        bi[b] += 1;
        lex[judge(d.lexical, c.answer)] += 1;
        tier_only += usize::from(t == 0 && b != 0);
        builtin_only += usize::from(b == 0 && t != 0);
    }
    let n_builtin = builtin.values().filter(|d| d.fold == test).count();
    assert_eq!(n_builtin, test_set.len(), "held-out sets differ from rosfix-train's");
    let kind = if base.is_some() { "residual" } else { "plain" };
    let path = out.join(format!("{}_{kind}_fold{test}.rfm", o.tier.name()));
    let sha = rosfix_model::format::save(&path, &model).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let r = serde_json::json!({
        "tier": o.tier.name(), "fold": test, "residual": base.is_some(), "n": test_set.len(), "n_calib": cal_cases.len(),
        "train_cases": data.len(), "steps": step, "train_seconds": train_secs, "shift": shift,
        "tier_cwa": tier, "builtin_cwa": bi, "lexical_cwa": lex,
        "tier_only_correct": tier_only, "builtin_only_correct": builtin_only,
        "epochs": log, "file": path.display().to_string(), "sha256": sha,
    });
    eprintln!("fold {test}: tier {tier:?} builtin {bi:?} lexical {lex:?} (correct/abstained/wrong), shift {shift:.1}, {train_secs:.0} s");
    r
}

type BaseScores = BTreeMap<(u8, usize), Vec<f32>>;

fn add_base(scores: Vec<Vec<f32>>, base: impl Iterator<Item = Vec<f32>>) -> Vec<Vec<f32>> {
    scores
        .into_iter()
        .zip(base)
        .map(|(s, b)| s.iter().enumerate().map(|(j, x)| x + b.get(j).copied().unwrap_or(0.0)).collect())
        .collect()
}

fn read_base(path: &str) -> BaseScores {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap_or_else(|e| panic!("{path}: {e}"));
            let s = v["scores"].as_array().map(|a| a.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect()).unwrap_or_default();
            ((v["fold"].as_u64().unwrap_or(0) as u8, v["idx"].as_u64().unwrap_or(0) as usize), s)
        })
        .collect()
}

pub struct Decision {
    fold: u8,
    target: String,
    current: String,
    builtin: Option<usize>,
    lexical: Option<usize>,
}

fn read_decisions(path: &str) -> BTreeMap<usize, Decision> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap_or_else(|e| panic!("{path}: {e}"));
            let idx = v["idx"].as_u64().expect("idx") as usize;
            let d = Decision {
                fold: v["fold"].as_u64().unwrap_or(0) as u8,
                target: v["target"].as_str().unwrap_or("").to_string(),
                current: v["current"].as_str().unwrap_or("").to_string(),
                builtin: v["builtin"].as_u64().map(|x| x as usize),
                lexical: v["lexical"].as_u64().map(|x| x as usize),
            };
            (idx, d)
        })
        .collect()
}

const FOLDS: u8 = 5;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: rosfix-tier stats CASES.jsonl MERGES.txt\n       rosfix-tier train --cases CASES.jsonl --merges MERGES.txt --decisions HELD_OUT.jsonl --out DIR [--tier nano] [--folds 0,1,2,3,4] [--epochs N] [--batch N] [--lr X] [--seed N] [--max-steps N] [--base-scores SCORES.jsonl]";
    let load_tok = |merges: &str| {
        let text = std::fs::read_to_string(merges).unwrap_or_else(|e| panic!("{merges}: {e}"));
        Tokenizer::from_text(&text).unwrap_or_else(|e| panic!("{merges}: {e}"))
    };
    match args.first().map(String::as_str) {
        Some("stats") if args.len() == 3 => stats(&read_cases(&args[1]), &load_tok(&args[2])),
        Some("train") => {
            let mut kv: BTreeMap<String, String> = BTreeMap::new();
            let mut it = args[1..].iter();
            while let (Some(k), Some(v)) = (it.next(), it.next()) {
                kv.insert(k.trim_start_matches("--").to_string(), v.clone());
            }
            let get = |k: &str| kv.get(k).cloned();
            let need = |k: &str| get(k).unwrap_or_else(|| {
                eprintln!("{usage}");
                std::process::exit(2)
            });
            let o = Opts {
                tier: Tier::parse(&get("tier").unwrap_or("nano".into())).expect("tier: nano|mini|base"),
                epochs: get("epochs").and_then(|v| v.parse().ok()).unwrap_or(3),
                batch: get("batch").and_then(|v| v.parse().ok()).unwrap_or(32),
                lr: get("lr").and_then(|v| v.parse().ok()).unwrap_or(1e-3),
                seed: get("seed").and_then(|v| v.parse().ok()).unwrap_or(1),
                max_steps: get("max-steps").and_then(|v| v.parse().ok()).unwrap_or(usize::MAX),
            };
            let folds: Vec<u8> = get("folds")
                .unwrap_or("0,1,2,3,4".into())
                .split(',')
                .filter_map(|x| x.trim().parse().ok())
                .collect();
            let cases = read_cases(&need("cases"));
            let tok = load_tok(&need("merges"));
            let builtin = read_decisions(&need("decisions"));
            let base = get("base-scores").map(|p| read_base(&p));
            let out = PathBuf::from(need("out"));
            std::fs::create_dir_all(&out).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
            let results = out.join(format!("{}_{}_results.jsonl", o.tier.name(), if base.is_some() { "residual" } else { "plain" }));
            for k in folds {
                let r = run_fold(&cases, &builtin, base.as_ref(), &tok, k, &o, &out);
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&results)
                    .unwrap_or_else(|e| panic!("{}: {e}", results.display()));
                writeln!(f, "{r}").expect("write results");
            }
        }
        _ => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    }
}
