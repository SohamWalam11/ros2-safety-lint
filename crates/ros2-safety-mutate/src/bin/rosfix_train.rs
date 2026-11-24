//! rosfix-train: trains the built-in fix ranker (`rosfix::assist::learned`) in plain Rust.
//!
//! Data:
//! - **synthetic** (always): the real parameter names declared by upstream nodes, taken from
//!   the embedded knowledge packs (Nav2, ros2_controllers, slam_toolbox, …), corrupted the way
//!   people get them wrong (typos, abbreviations, dropped/swapped words, camelCase, plurals).
//!   The options are the other parameters of the same node, exactly as rosfix offers them.
//!   Some cases deliberately leave the right answer out, so the ranker learns to answer
//!   "none of these" instead of picking the least bad option.
//! - **JSONL** (optional, `--data f.jsonl`): `{"current", "options", "answer", "repo"}` lines,
//!   e.g. from `rosfix-bench --emit-dataset` or `training/mine_fixes.py`.
//!
//! Evaluation is on held-out packages/repositories **and names**: a held-out case whose name
//! also occurs in training is dropped (packages share names, e.g. Nav2 base-class parameters).
//! It is compared against the lexical ranker with the same abstention rule rosfix uses at
//! runtime, and repeated over 5 package folds (mean ± sd). Fold 0 is the shipped model. The
//! weights are written only if, summed over all folds, the learned ranker is correct more often
//! than the lexical one and makes no more wrong edits (`--force` overrides).
//!
//! Usage: rosfix-train [--data f.jsonl]... [--epochs N] [--seed N] [--out learned_weights.rs] [--force]

use rosfix::assist::learned::{features, Weights, N_FEATURES, N_HIDDEN};
use rosfix::assist::{decide, decide_lexical, lexical, TOP_K};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::PathBuf;

// ---------------------------------------------------------------------------- rng

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn normal(&mut self) -> f32 {
        let (u, v) = (self.unit().max(1e-7), self.unit());
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }
}

// ---------------------------------------------------------------------------- corruptions

const LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyz_";

fn typo(s: &str, rng: &mut Rng) -> String {
    let mut c: Vec<char> = s.chars().collect();
    if c.len() < 3 {
        return s.to_string();
    }
    let i = 1 + rng.below(c.len() - 1);
    match rng.below(4) {
        0 => {
            c.remove(i);
        }
        1 => c.insert(i, LETTERS[rng.below(LETTERS.len() - 1)] as char),
        2 => c[i] = LETTERS[rng.below(LETTERS.len() - 1)] as char,
        _ => {
            if i + 1 < c.len() {
                c.swap(i, i + 1);
            } else {
                c.swap(i - 1, i);
            }
        }
    }
    c.into_iter().collect()
}

fn words(s: &str) -> Vec<String> {
    s.split('_').map(str::to_string).collect()
}

fn abbreviate(s: &str, rng: &mut Rng) -> String {
    let mut w = words(s);
    let idx: Vec<usize> = (0..w.len()).filter(|i| w[*i].len() >= 4).collect();
    if idx.is_empty() {
        return s.to_string();
    }
    for _ in 0..1 + rng.below(2) {
        let i = idx[rng.below(idx.len())];
        let full = w[i].clone();
        let known: Vec<&str> = lexical::ABBREV
            .iter()
            .filter(|(a, f)| *f == full && a.len() < full.len())
            .map(|(a, _)| *a)
            .collect();
        w[i] = if !known.is_empty() && rng.below(3) > 0 {
            known[rng.below(known.len())].to_string()
        } else {
            full.chars().take(3 + rng.below(2)).collect()
        };
    }
    w.join("_")
}

fn drop_word(s: &str, rng: &mut Rng) -> String {
    let mut w = words(s);
    if w.len() < 3 {
        return s.to_string();
    }
    w.remove(rng.below(w.len()));
    w.join("_")
}

fn swap_words(s: &str, rng: &mut Rng) -> String {
    let mut w = words(s);
    if w.len() < 2 {
        return s.to_string();
    }
    let i = rng.below(w.len() - 1);
    w.swap(i, i + 1);
    w.join("_")
}

fn camel(s: &str, _rng: &mut Rng) -> String {
    let mut out = String::new();
    for (i, w) in words(s).iter().enumerate() {
        let mut ch = w.chars();
        match (i, ch.next()) {
            (0, Some(f)) => {
                out.push(f);
                out.extend(ch);
            }
            (_, Some(f)) => {
                out.extend(f.to_uppercase());
                out.extend(ch);
            }
            _ => {}
        }
    }
    out
}

fn plural(s: &str, rng: &mut Rng) -> String {
    let mut w = words(s);
    let i = rng.below(w.len());
    if w[i].ends_with('s') && w[i].len() > 3 {
        w[i].pop();
    } else {
        w[i].push('s');
    }
    w.join("_")
}

fn abbrev_typo(s: &str, rng: &mut Rng) -> String {
    let a = abbreviate(s, rng);
    typo(&a, rng)
}

fn double_typo(s: &str, rng: &mut Rng) -> String {
    let a = typo(s, rng);
    typo(&a, rng)
}

type Op = (&'static str, fn(&str, &mut Rng) -> String);
const OPS: &[Op] = &[
    ("typo", typo),
    ("double_typo", double_typo),
    ("abbreviation", abbreviate),
    ("abbrev+typo", abbrev_typo),
    ("dropped_word", drop_word),
    ("swapped_words", swap_words),
    ("camelCase", camel),
    ("plural", plural),
];

// ---------------------------------------------------------------------------- data

struct Case {
    op: String,
    /// Fold 0..FOLDS by package / repository. A run uses one fold as held-out test, the
    /// next one for calibration and the rest for training (fold 0 = the shipped model).
    fold: u8,
    /// The real name the case was made from (the intended answer, or the name whose
    /// corruption is offered without it). Evaluation drops held-out cases whose name also
    /// occurs in training: packages share names (e.g. every Nav2 server declares
    /// `action_server_result_timeout`), so package-disjoint alone is not name-disjoint.
    target: String,
    current: String,
    options: Vec<String>,
    /// Index of the right option, or None when the right answer is not among the options
    /// (then every pick is a wrong edit and the ranker must answer "none of these").
    answer: Option<usize>,
    x: Vec<[f32; N_FEATURES]>,
}

fn make_case(
    op: &str,
    fold: u8,
    target: &str,
    current: String,
    pool: &[String],
    answer: Option<&str>,
) -> Option<Case> {
    if pool.is_empty() || pool.contains(&current) {
        return None; // the "wrong" text is itself a valid name: not an error
    }
    let options: Vec<String> = lexical::rank(&current, pool)
        .into_iter()
        .take(TOP_K)
        .map(|x| x.0)
        .collect();
    let answer = match answer {
        Some(a) => Some(options.iter().position(|c| c == a)?),
        None => None,
    };
    let x = features(&current, &options);
    Some(Case {
        op: op.to_string(),
        fold,
        target: target.to_string(),
        current,
        options,
        answer,
        x,
    })
}

const FOLDS: u8 = 5;

fn fold_of(group: &str) -> u8 {
    (rosfix::util::fnv1a(group.as_bytes()) % FOLDS as u64) as u8
}

fn synthetic(rng: &mut Rng, per_name: usize) -> Vec<Case> {
    // (package, node) → parameter names; executables per package as a second kind of set.
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for distro in ["humble", "jazzy", "kilted"] {
        let Some(p) = rosfix::pack::embedded(distro) else {
            continue;
        };
        for (pkg, pp) in &p.packages {
            for (name, node) in pp.executables.iter().chain(pp.components.iter()) {
                let set = groups.entry(format!("{pkg}|{name}")).or_default();
                set.extend(
                    node.params
                        .iter()
                        .map(|x| x.name.literal().to_string())
                        .filter(|n| n.len() >= 3 && !n.contains('*')),
                );
            }
            let exes = groups.entry(format!("{pkg}|<executables>")).or_default();
            exes.extend(pp.executables.keys().cloned());
        }
    }
    let all_names: Vec<String> = groups
        .values()
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut cases = Vec::new();
    for (key, names) in &groups {
        if names.len() < 3 {
            continue;
        }
        let fold = fold_of(key.split('|').next().unwrap_or(""));
        let pool: Vec<String> = names.iter().cloned().collect();
        for target in &pool {
            for _ in 0..per_name {
                let (op, f) = OPS[rng.below(OPS.len())];
                let cur = f(target, rng);
                if cur == *target {
                    continue;
                }
                cases.extend(make_case(
                    op,
                    fold,
                    target,
                    cur.clone(),
                    &pool,
                    Some(target),
                ));
                // Same mistake, but the intended name is not offered (e.g. it belongs to
                // another node or another distribution): the ranker must abstain.
                if rng.below(2) == 0 {
                    let others: Vec<String> =
                        pool.iter().filter(|p| *p != target).cloned().collect();
                    cases.extend(make_case(
                        "absent: siblings only",
                        fold,
                        target,
                        cur,
                        &others,
                        None,
                    ));
                }
            }
            // A name from somewhere else entirely (like a remap of a topic the node never uses).
            if rng.below(2) == 0 {
                let stranger = all_names[rng.below(all_names.len())].clone();
                cases.extend(make_case(
                    "absent: unrelated name",
                    fold,
                    &stranger,
                    stranger.clone(),
                    &pool,
                    None,
                ));
            }
        }
    }
    cases
}

fn jsonl(path: &PathBuf) -> Vec<Case> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let (Some(cur), Some(opts), Some(ans)) = (
            v["current"].as_str(),
            v["options"].as_array(),
            v["answer"].as_u64(),
        ) else {
            continue;
        };
        let options: Vec<String> = opts
            .iter()
            .filter_map(|o| o.as_str().map(str::to_string))
            .collect();
        let Some(answer) = options.get(ans as usize).cloned() else {
            continue;
        };
        let repo = v["repo"].as_str().unwrap_or("?");
        let op = v["op"].as_str().unwrap_or("jsonl");
        out.extend(make_case(
            &format!("data:{op}"),
            fold_of(repo),
            &answer,
            cur.to_string(),
            &options,
            Some(&answer),
        ));
    }
    out
}

// ---------------------------------------------------------------------------- training

/// Index of a parameter in the flat (w1, b1, w2, b2) vector.
const P_B1: usize = N_HIDDEN * N_FEATURES;
const P_W2: usize = P_B1 + N_HIDDEN;
const P_B2: usize = P_W2 + N_HIDDEN;
const NP: usize = P_B2 + 1;

fn param(w: &mut Weights, k: usize) -> &mut f32 {
    match k {
        k if k < P_B1 => &mut w.w1[k / N_FEATURES][k % N_FEATURES],
        k if k < P_W2 => &mut w.b1[k - P_B1],
        k if k < P_B2 => &mut w.w2[k - P_W2],
        _ => &mut w.b2,
    }
}

/// Listwise training: softmax over the options plus the fixed "none of these" score.
fn train(cases: &[&Case], epochs: usize, rng: &mut Rng) -> Weights {
    let all: Vec<&[f32; N_FEATURES]> = cases.iter().flat_map(|c| c.x.iter()).collect();
    let n = all.len().max(1) as f32;
    let mut mean = [0.0f32; N_FEATURES];
    let mut std = [0.0f32; N_FEATURES];
    for x in &all {
        for i in 0..N_FEATURES {
            mean[i] += x[i] / n;
        }
    }
    for x in &all {
        for i in 0..N_FEATURES {
            std[i] += (x[i] - mean[i]).powi(2) / n;
        }
    }
    for s in &mut std {
        *s = s.sqrt().max(1e-3);
    }
    let mut w = Weights {
        mean,
        std,
        w1: [[0.0; N_FEATURES]; N_HIDDEN],
        b1: [0.0; N_HIDDEN],
        w2: [0.0; N_HIDDEN],
        b2: 0.0,
    };
    let scale = 1.0 / (N_FEATURES as f32).sqrt();
    for row in &mut w.w1 {
        for v in row.iter_mut() {
            *v = rng.normal() * scale;
        }
    }
    for v in &mut w.w2 {
        *v = rng.normal() * 0.25;
    }

    let (mut m, mut v) = (vec![0.0f32; NP], vec![0.0f32; NP]);
    let (lr, beta1, beta2, eps, decay) = (0.01f32, 0.9f32, 0.999f32, 1e-8f32, 1e-4f32);
    let mut order: Vec<usize> = (0..cases.len()).collect();
    let mut t = 0i32;
    for epoch in 0..epochs {
        for i in (1..order.len()).rev() {
            order.swap(i, rng.below(i + 1));
        }
        let mut loss = 0.0f32;
        for chunk in order.chunks(32) {
            let mut g = vec![0.0f32; NP];
            for &ci in chunk {
                let c = cases[ci];
                let fw: Vec<_> = c.x.iter().map(|x| w.forward(x)).collect();
                let mut s: Vec<f32> = fw.iter().map(|f| f.2).collect();
                s.push(rosfix::assist::NONE_SCORE);
                let p = rosfix::assist::softmax(&s);
                let target = c.answer.unwrap_or(c.options.len());
                loss -= p[target].max(1e-9).ln();
                for (k, (z, h, _)) in fw.iter().enumerate() {
                    let gs = p[k] - f32::from(k == target);
                    g[P_B2] += gs;
                    for j in 0..N_HIDDEN {
                        g[P_W2 + j] += gs * h[j];
                        let dh = gs * w.w2[j] * (1.0 - h[j] * h[j]);
                        g[P_B1 + j] += dh;
                        for f in 0..N_FEATURES {
                            g[j * N_FEATURES + f] += dh * z[f];
                        }
                    }
                }
            }
            t += 1;
            let bs = chunk.len() as f32;
            for (k, gk) in g.iter().enumerate() {
                let p = param(&mut w, k);
                let gk = gk / bs + if k == P_B2 { 0.0 } else { decay * *p };
                m[k] = beta1 * m[k] + (1.0 - beta1) * gk;
                v[k] = beta2 * v[k] + (1.0 - beta2) * gk * gk;
                let mh = m[k] / (1.0 - beta1.powi(t));
                let vh = v[k] / (1.0 - beta2.powi(t));
                *p -= lr * mh / (vh.sqrt() + eps);
            }
        }
        if epoch % 5 == 0 || epoch + 1 == epochs {
            eprintln!(
                "epoch {epoch:>3}  loss {:.4}",
                loss / cases.len().max(1) as f32
            );
        }
    }
    w
}

// ---------------------------------------------------------------------------- evaluation

/// [correct, abstained, wrong] per ranker. For cases without a right answer, "abstained"
/// is the correct outcome and every pick counts as wrong.
#[derive(Default, Clone, Copy)]
struct Tally {
    n: usize,
    lex: [usize; 3],
    learned: [usize; 3],
}

impl Tally {
    /// Wrong edits per case (both kinds of case together): the number that must not grow.
    fn wrong(a: [usize; 3]) -> usize {
        a[2]
    }
}

fn judge(pick: Option<usize>, answer: Option<usize>) -> usize {
    match (pick, answer) {
        (Some(p), Some(a)) if p == a => 0,
        (None, _) => 1,
        _ => 2,
    }
}

fn evaluate(w: &Weights, cases: &[&Case]) -> BTreeMap<String, Tally> {
    let mut t: BTreeMap<String, Tally> = BTreeMap::new();
    for c in cases {
        let scores: Vec<f32> = c.x.iter().map(|x| w.forward(x).2).collect();
        let l = judge(decide(&scores).map(|x| x.0), c.answer);
        let x = judge(decide_lexical(&c.current, &c.options), c.answer);
        let group = if c.answer.is_some() {
            "ALL (answer offered)"
        } else {
            "ALL (answer absent)"
        };
        for key in [c.op.clone(), group.to_string(), "ALL".to_string()] {
            let e = t.entry(key).or_default();
            e.n += 1;
            e.learned[l] += 1;
            e.lex[x] += 1;
        }
    }
    t
}

struct FoldRun {
    weights: Weights,
    shift: f32,
    n_train: usize,
    n_calib: usize,
    n_test: usize,
    /// Held-out cases dropped because their name also occurs in training.
    n_test_leaked: usize,
    /// Name-disjoint held-out result (what the gate and the report use).
    result: BTreeMap<String, Tally>,
    /// Package-disjoint only (the old measure), for comparison.
    leaky: BTreeMap<String, Tally>,
}

/// Trains on all folds except `test` and `test + 1`, calibrates on `test + 1`, evaluates on
/// `test`. Calibration and test keep only cases whose name never occurs in training.
fn run_fold(cases: &[Case], test: u8, epochs: usize, rng: &mut Rng) -> FoldRun {
    let calib = (test + 1) % FOLDS;
    let train_set: Vec<&Case> = cases
        .iter()
        .filter(|c| c.fold != test && c.fold != calib)
        .collect();
    let seen: BTreeSet<&str> = train_set.iter().map(|c| c.target.as_str()).collect();
    let unseen = |fold: u8| -> Vec<&Case> {
        cases
            .iter()
            .filter(|c| c.fold == fold && !seen.contains(c.target.as_str()))
            .collect()
    };
    let calib_set = unseen(calib);
    let test_all: Vec<&Case> = cases.iter().filter(|c| c.fold == test).collect();
    let test_set = unseen(test);

    let mut w = train(&train_set, epochs, rng);

    // Calibration on unseen packages and names: lower every score (i.e. make "none of
    // these" more attractive) until the learned ranker makes no more wrong edits than the
    // lexical ranker.
    let base_b2 = w.b2;
    let mut shift = 0.0f32;
    for k in 0..=40 {
        shift = k as f32 * 0.1;
        w.b2 = base_b2 - shift;
        let t = evaluate(&w, &calib_set)["ALL"];
        if Tally::wrong(t.learned) <= Tally::wrong(t.lex) {
            break;
        }
    }
    FoldRun {
        result: evaluate(&w, &test_set),
        leaky: evaluate(&w, &test_all),
        n_train: train_set.len(),
        n_calib: calib_set.len(),
        n_test: test_set.len(),
        n_test_leaked: test_all.len() - test_set.len(),
        weights: w,
        shift,
    }
}

fn render(w: &Weights, note: &str) -> String {
    let arr = |v: &[f32]| {
        v.iter()
            .map(|x| format!("{x:.6}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut s = String::new();
    let _ = writeln!(
        s,
        "// Generated by `rosfix-train` — do not edit by hand.\n// {note}"
    );
    let _ = writeln!(
        s,
        "#[allow(clippy::excessive_precision)]\nstatic BUILTIN: Weights = Weights {{"
    );
    let _ = writeln!(s, "    mean: [{}],", arr(&w.mean));
    let _ = writeln!(s, "    std: [{}],", arr(&w.std));
    let _ = writeln!(s, "    w1: [");
    for row in &w.w1 {
        let _ = writeln!(s, "        [{}],", arr(row));
    }
    let _ = writeln!(s, "    ],");
    let _ = writeln!(s, "    b1: [{}],", arr(&w.b1));
    let _ = writeln!(s, "    w2: [{}],", arr(&w.w2));
    let _ = writeln!(s, "    b2: {:.6},", w.b2);
    let _ = writeln!(s, "}};");
    s
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut data = Vec::new();
    let mut epochs = 40usize;
    let mut seed = 7u64;
    let mut per_name = 3usize;
    let mut force = false;
    let mut out = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ros2-safety-lint/src/assist/learned_weights.rs"
    ));
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().unwrap_or_default();
        match a.as_str() {
            "--data" => data.push(PathBuf::from(val())),
            "--epochs" => epochs = val().parse().unwrap_or(epochs),
            "--seed" => seed = val().parse().unwrap_or(seed),
            "--per-name" => per_name = val().parse().unwrap_or(per_name),
            "--out" => out = PathBuf::from(val()),
            "--force" => force = true,
            _ => {
                eprintln!("usage: rosfix-train [--data f.jsonl]... [--epochs N] [--seed N] [--per-name N] [--out file.rs] [--force]");
                std::process::exit(2);
            }
        }
    }
    let mut rng = Rng(seed);
    let mut cases = synthetic(&mut rng, per_name);
    for d in &data {
        cases.extend(jsonl(d));
    }
    // Fold 0 is the shipped model; folds 1.. only measure how much the result depends on
    // which packages happen to be held out.
    let runs: Vec<FoldRun> = (0..FOLDS)
        .map(|k| {
            let mut fold_rng = if k == 0 {
                Rng(rng.0)
            } else {
                Rng(seed ^ (0x9e37_79b9 * k as u64))
            };
            run_fold(&cases, k, epochs, &mut fold_rng)
        })
        .collect();
    let shipped = &runs[0];
    eprintln!(
        "fold 0: {} training, {} calibration, {} held-out cases (disjoint packages; {} held-out \
         cases share their name with training and are excluded), calibration shift -{:.1}",
        shipped.n_train, shipped.n_calib, shipped.n_test, shipped.n_test_leaked, shipped.shift
    );

    println!("Held-out packages, name-disjoint (fold 0) — correct / abstained / wrong (for \"absent\" cases abstaining is correct, any pick is wrong):\n");
    println!("| case | n | lexical | learned |\n|---|---:|---|---|");
    let f = |a: [usize; 3]| format!("{} / {} / {}", a[0], a[1], a[2]);
    for (op, t) in &shipped.result {
        println!("| {op} | {} | {} | {} |", t.n, f(t.lex), f(t.learned));
    }
    let leaky = shipped.leaky.get("ALL").copied().unwrap_or_default();
    println!(
        "\nFor comparison, package-disjoint only (names shared with training kept): n {}, lexical {}, learned {}",
        leaky.n,
        f(leaky.lex),
        f(leaky.learned)
    );

    // Cross-validation over package folds, name-disjoint, as rates per held-out case.
    println!("\n{FOLDS}-fold cross-validation (name-disjoint held-out cases):\n");
    println!("| fold | n | lexical correct | learned correct | lexical wrong | learned wrong |\n|---|---:|---:|---:|---:|---:|");
    let mut rates: [Vec<f64>; 4] = Default::default();
    let mut total = Tally::default();
    for (k, r) in runs.iter().enumerate() {
        let t = r.result.get("ALL").copied().unwrap_or_default();
        let n = t.n.max(1) as f64;
        let v = [
            t.lex[0] as f64 / n,
            t.learned[0] as f64 / n,
            t.lex[2] as f64 / n,
            t.learned[2] as f64 / n,
        ];
        println!(
            "| {k} | {} | {:.1}% | {:.1}% | {:.2}% | {:.2}% |",
            t.n,
            100.0 * v[0],
            100.0 * v[1],
            100.0 * v[2],
            100.0 * v[3]
        );
        for (dst, x) in rates.iter_mut().zip(v) {
            dst.push(x);
        }
        total.n += t.n;
        for i in 0..3 {
            total.lex[i] += t.lex[i];
            total.learned[i] += t.learned[i];
        }
    }
    let ms = |v: &[f64]| {
        let m = v.iter().sum::<f64>() / v.len() as f64;
        let sd = (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() as f64 - 1.0).max(1.0))
            .sqrt();
        format!("{:.1} ± {:.1}%", 100.0 * m, 100.0 * sd)
    };
    let summary = format!(
        "correct {} vs lexical {}, wrong {} vs lexical {}",
        ms(&rates[1]),
        ms(&rates[0]),
        ms(&rates[3]),
        ms(&rates[2])
    );
    println!(
        "| mean ± sd | | {} | {} | {} | {} |",
        ms(&rates[0]),
        ms(&rates[1]),
        ms(&rates[2]),
        ms(&rates[3])
    );
    println!(
        "\nmodel: {NP} weights + {} normalisation constants ({:.1} KB as f32)",
        2 * N_FEATURES,
        (NP + 2 * N_FEATURES) as f64 * 4.0 / 1024.0
    );

    // Ship only if, over all folds' name-disjoint held-out cases, the learned ranker is right
    // more often AND makes no more wrong edits than the lexical one.
    let ok =
        total.learned[0] > total.lex[0] && Tally::wrong(total.learned) <= Tally::wrong(total.lex);
    if !ok && !force {
        eprintln!("not written: across {FOLDS} folds the learned ranker does not beat the lexical ranker on name-disjoint held-out cases (use --force)");
        std::process::exit(1);
    }
    let w = shipped.weights.clone();
    let all = shipped.result.get("ALL").copied().unwrap_or_default();
    let note = format!(
        "seed {seed}, {epochs} epochs, fold 0: {} train / {} name-disjoint held-out cases; held-out correct {} vs lexical {}, wrong edits {} vs lexical {}\n\
         // {FOLDS}-fold CV (name-disjoint): {summary}",
        shipped.n_train,
        all.n,
        all.learned[0],
        all.lex[0],
        all.learned[2],
        all.lex[2]
    );
    std::fs::write(&out, render(&w, &note)).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    eprintln!("wrote {}", out.display());
}
