//! rosfix-bench: mutation benchmark on real ROS 2 workspaces.
//!
//! For every workspace given on the command line:
//! 1. Analyse the clean workspace and record every finding (precision side: these are
//!    candidates for manual labelling — the clean corpus is not assumed bug-free).
//! 2. Discover real mutation sites from the clean analysis (e.g. a parameter that a node
//!    really declares and that a launch file really binds).
//! 3. Inject one realistic misconfiguration at a time **in memory** (files on disk are
//!    never modified) and re-run the full analysis.
//! 4. A mutation counts as detected only if the *expected rule* fires *at the mutated
//!    line* (or, for dependency removal, names the removed dependency).
//!
//! Usage: rosfix-bench <workspace>... [--max-per-op N] [--out results.md]

use rosfix::assist::lexical;
use rosfix::config::Config;
use rosfix::engine::{analyze, Analysis, Options};
use rosfix::facts::{NamePattern, ParamType, ParamValue};
use rosfix::resolve::{declared_params, is_load_marker, ImplStatus};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

struct Mutation {
    op: &'static str,
    expect_rule: &'static str,
    file: PathBuf,
    rel: String,
    new_text: String,
    /// 1-based line where the finding is expected, if line-precise.
    line: Option<usize>,
    /// Alternatively: substrings the finding's message must contain.
    message_has: Option<String>,
    message_pkg: Option<String>,
    /// The correct text at the repair site (for assistant accuracy and training data).
    original: Option<String>,
    what: String,
}

/// Fix-assistant accuracy: [correct, abstained, wrong].
#[derive(Default)]
struct AssistStats {
    cases: usize,
    /// The right answer was among the options shown to the assistant.
    reachable: usize,
    lexical: [usize; 3],
    model: [usize; 3],
}

#[derive(Default)]
struct OpStats {
    sites: usize,
    detected: usize,
    missed: Vec<String>,
}

/// Re-analyses one mutation fully and with the parse cache only, timing each, and counts a
/// mismatch if the findings differ from the incremental (cache + reuse) result `inc`.
/// Pushes [full ms, cache-only ms, incremental ms, mismatch 0/1].
fn ablate(
    root: &Path,
    config: &Config,
    m: &Mutation,
    inc: &Analysis,
    cache: &std::sync::Arc<rosfix::workspace::ExtractCache>,
    out: &mut Vec<[u128; 4]>,
) {
    let key = |a: &Analysis| {
        let mut v: Vec<(String, &'static str, usize, String)> = a
            .diagnostics
            .iter()
            .map(|d| {
                (
                    a.index.file(d.loc.file).rel_path.clone(),
                    d.rule,
                    d.loc.start,
                    d.message.clone(),
                )
            })
            .collect();
        v.sort();
        v
    };
    let total = |a: &Analysis| a.timings.index_ms + a.timings.resolve_ms + a.timings.agents_ms;
    let mut full_o = Options::default();
    full_o.overrides.insert(m.file.clone(), m.new_text.clone());
    let full = analyze(root, config, &full_o);
    let mut cache_o = Options {
        cache: Some(cache.clone()),
        ..Default::default()
    };
    cache_o.overrides.insert(m.file.clone(), m.new_text.clone());
    let cached = analyze(root, config, &cache_o);
    let mismatch = u128::from(key(&full) != key(inc) || key(&full) != key(&cached));
    out.push([total(&full), total(&cached), total(inc), mismatch]);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut roots = Vec::new();
    let mut max_per_op = 10usize;
    let mut out_path: Option<PathBuf> = None;
    let mut dataset_path: Option<PathBuf> = None;
    let mut ablation = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--max-per-op" => {
                i += 1;
                max_per_op = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(10);
            }
            "--out" => {
                i += 1;
                out_path = args.get(i).map(PathBuf::from);
            }
            "--ablation" => ablation = true,
            "--emit-dataset" => {
                i += 1;
                dataset_path = args.get(i).map(PathBuf::from);
            }
            "-h" | "--help" => {
                println!("usage: rosfix-bench <workspace>... [--max-per-op N] [--out results.md]");
                return;
            }
            other => roots.push(PathBuf::from(other)),
        }
        i += 1;
    }
    if roots.is_empty() {
        eprintln!("usage: rosfix-bench <workspace>... [--max-per-op N] [--out results.md]");
        std::process::exit(2);
    }

    let config = Config::default();
    let mut dataset = dataset_path
        .as_ref()
        .map(|p| std::io::BufWriter::new(std::fs::File::create(p).expect("create dataset file")));
    let model = rosfix::assist::load_model();
    let mut assist: BTreeMap<&'static str, AssistStats> = BTreeMap::new();
    let mut ablation_rows: Vec<String> = Vec::new();
    let mut report = String::from("# rosfix mutation benchmark\n\n");
    let mut totals: BTreeMap<&'static str, OpStats> = BTreeMap::new();
    let mut clean_totals: BTreeMap<&'static str, usize> = BTreeMap::new();

    for root in &roots {
        let name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let t = Instant::now();
        // Each mutation is a change event against the clean run: only the mutated file is
        // parsed again and only agents that read that kind of file run again.
        let cache = std::sync::Arc::new(rosfix::workspace::ExtractCache::default());
        let clean = analyze(
            root,
            &config,
            &Options {
                cache: Some(cache.clone()),
                ..Default::default()
            },
        );
        let previous = std::sync::Arc::new(clean.snapshot());
        let clean_ms = t.elapsed().as_millis();
        eprintln!(
            "[{name}] clean analysis: {} findings in {clean_ms} ms",
            clean.diagnostics.len()
        );

        let mut by_rule: BTreeMap<&'static str, usize> = BTreeMap::new();
        for d in &clean.diagnostics {
            *by_rule.entry(d.rule).or_default() += 1;
            *clean_totals.entry(d.rule).or_default() += 1;
        }

        let muts = discover(&clean, max_per_op);
        // Cost of one re-analysis (index, resolve, agents in ms; files parsed; agents rerun).
        let mut cost = [0u128; 6];
        let mut ablations: Vec<[u128; 4]> = Vec::new();
        let mut stats: BTreeMap<&'static str, OpStats> = BTreeMap::new();
        for m in &muts {
            let mut o = Options {
                cache: Some(cache.clone()),
                previous: Some(previous.clone()),
                ..Default::default()
            };
            o.overrides.insert(m.file.clone(), m.new_text.clone());
            let after = analyze(root, &config, &o);
            if ablation {
                ablate(root, &config, m, &after, &cache, &mut ablations);
            }
            cost[0] += after.timings.index_ms;
            cost[1] += after.timings.resolve_ms;
            cost[2] += after.timings.agents_ms;
            cost[3] += after.stats().files_parsed as u128;
            cost[4] += after.runs.iter().filter(|r| r.ran).count() as u128;
            cost[5] += 1;
            let hit = after.diagnostics.iter().any(|d| {
                if d.rule != m.expect_rule {
                    return false;
                }
                let f = after.index.file(d.loc.file);
                if let Some(line) = m.line {
                    f.rel_path == m.rel && f.line_col(d.loc.start).0 == line
                } else {
                    m.message_has
                        .as_ref()
                        .is_some_and(|s| d.message.contains(s.as_str()))
                        && m.message_pkg
                            .as_ref()
                            .is_none_or(|s| d.message.contains(s.as_str()))
                }
            });
            // ---- fix-assistant evaluation and training data (mutations with a known answer)
            if let Some(orig) = &m.original {
                let found = after.diagnostics.iter().enumerate().find(|(_, d)| {
                    let f = after.index.file(d.loc.file);
                    d.rule == m.expect_rule
                        && d.repair.is_some()
                        && f.rel_path == m.rel
                        && m.line.is_none_or(|l| f.line_col(d.loc.start).0 == l)
                });
                if let Some((di, d)) = found {
                    let r = d.repair.as_ref().expect("repair");
                    let top: Vec<String> = lexical::rank(&r.current, &r.candidates)
                        .into_iter()
                        .take(rosfix::assist::TOP_K)
                        .map(|x| x.0)
                        .collect();
                    let a = assist.entry(m.op).or_default();
                    a.cases += 1;
                    let pos = top.iter().position(|c| c == orig);
                    a.reachable += usize::from(pos.is_some());
                    let judge = |props: &[rosfix::assist::Proposal]| match props
                        .iter()
                        .find(|p| p.diag == di)
                    {
                        Some(p) if p.value == *orig => 0usize,
                        Some(_) => 2,
                        None => 1,
                    };
                    a.lexical[judge(&rosfix::assist::propose(&after, None))] += 1;
                    a.model[judge(&rosfix::assist::propose(&after, Some(model.as_ref())))] += 1;
                    if let (Some(w), Some(answer)) = (dataset.as_mut(), pos) {
                        let line = serde_json::json!({
                            "current": r.current,
                            "options": top,
                            "answer": answer,
                            "rule": d.rule,
                            "op": m.op,
                            "repo": name,
                        });
                        let _ = writeln!(w, "{line}");
                    }
                }
            }
            let s = stats.entry(m.op).or_default();
            s.sites += 1;
            if hit {
                s.detected += 1;
            } else {
                s.missed.push(format!("{} ({})", m.what, m.rel));
            }
            eprintln!(
                "[{name}] {:<18} {} {}",
                m.op,
                if hit { "detected" } else { "MISSED  " },
                m.what
            );
        }
        if !ablations.is_empty() {
            let mut cols: Vec<Vec<u128>> = (0..3)
                .map(|k| ablations.iter().map(|a| a[k]).collect())
                .collect();
            for c in &mut cols {
                c.sort();
            }
            let med = |c: &Vec<u128>| c[c.len() / 2];
            let mismatches: u128 = ablations.iter().map(|a| a[3]).sum();
            let line = format!(
                "| {name} | {} | {} | {} | {} | {mismatches} |",
                ablations.len(),
                med(&cols[0]),
                med(&cols[1]),
                med(&cols[2])
            );
            eprintln!("[{name}] ablation (median ms per re-analysis): full {}, cache only {}, cache+reuse {}; result mismatches {mismatches}", med(&cols[0]), med(&cols[1]), med(&cols[2]));
            ablation_rows.push(line);
        }
        if cost[5] > 0 {
            let n = cost[5];
            eprintln!(
                "[{name}] per re-analysis (mean of {n}): index {} ms, resolve {} ms, agents {} ms; {:.1} files parsed, {:.1} agents run",
                cost[0] / n,
                cost[1] / n,
                cost[2] / n,
                cost[3] as f64 / n as f64,
                cost[4] as f64 / n as f64
            );
        }

        let _ = writeln!(
            report,
            "## {name}\n\nClean analysis: {} findings, {clean_ms} ms ({} files parsed).\n",
            clean.diagnostics.len(),
            clean.index.stats.files_parsed
        );
        if !by_rule.is_empty() {
            let _ = writeln!(
                report,
                "Findings on the clean workspace (to be labelled manually): {}\n",
                by_rule
                    .iter()
                    .map(|(r, n)| format!("{r}={n}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let _ = writeln!(
            report,
            "| mutation | expected rule | injected | detected | recall |\n|---|---|---:|---:|---:|"
        );
        for (op, s) in &stats {
            let rule = muts
                .iter()
                .find(|m| m.op == *op)
                .map(|m| m.expect_rule)
                .unwrap_or("?");
            let _ = writeln!(
                report,
                "| {op} | {rule} | {} | {} | {:.0}% |",
                s.sites,
                s.detected,
                100.0 * s.detected as f64 / s.sites.max(1) as f64
            );
            let tot = totals.entry(op).or_default();
            tot.sites += s.sites;
            tot.detected += s.detected;
            tot.missed
                .extend(s.missed.iter().map(|x| format!("[{name}] {x}")));
        }
        if stats.is_empty() {
            let _ = writeln!(report, "| (no mutation sites found) | | 0 | 0 | |");
        }
        report.push('\n');
    }

    let _ = writeln!(
        report,
        "## Total\n\n| mutation | injected | detected | recall |\n|---|---:|---:|---:|"
    );
    let (mut s_all, mut d_all) = (0, 0);
    for (op, s) in &totals {
        s_all += s.sites;
        d_all += s.detected;
        let _ = writeln!(
            report,
            "| {op} | {} | {} | {:.0}% |",
            s.sites,
            s.detected,
            100.0 * s.detected as f64 / s.sites.max(1) as f64
        );
    }
    let _ = writeln!(
        report,
        "| **all** | **{s_all}** | **{d_all}** | **{:.0}%** |\n",
        100.0 * d_all as f64 / s_all.max(1) as f64
    );
    if !assist.is_empty() {
        let _ = writeln!(
            report,
            "## Fix assistant\n\nFor mutations with a known correct value: how often the right value was among the options, \
             and what the lexical ranker and the built-in learned ranker chose (correct / abstained / wrong). \
             Abstaining is safe: the finding is still reported.\n\n\
             | mutation | cases | answer among options | lexical | learned |\n|---|---:|---:|---|---|"
        );
        for (op, s) in &assist {
            let m = format!("{} / {} / {}", s.model[0], s.model[1], s.model[2]);
            let _ = writeln!(
                report,
                "| {op} | {} | {} | {} / {} / {} | {m} |",
                s.cases, s.reachable, s.lexical[0], s.lexical[1], s.lexical[2]
            );
        }
        report.push('\n');
    }
    if let Some(mut w) = dataset.take() {
        let _ = w.flush();
        eprintln!(
            "wrote training examples to {}",
            dataset_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        );
    }
    let _ = writeln!(
        report,
        "Findings on clean workspaces by rule: {}\n",
        clean_totals
            .iter()
            .map(|(r, n)| format!("{r}={n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if !ablation_rows.is_empty() {
        let _ = writeln!(
            report,
            "## Incremental re-analysis (ablation)

Each mutation re-analysed three ways; median wall time in ms. \"cache + reuse\" adds agent and launch-graph reuse to the parse cache. Mismatches = re-analyses whose findings differ from the full analysis (must be 0).

| workspace | re-analyses | full | parse cache only | cache + reuse | mismatches |
|---|---:|---:|---:|---:|---:|"
        );
        for r in &ablation_rows {
            let _ = writeln!(report, "{r}");
        }
        report.push('\n');
    }
    let missed: Vec<&String> = totals.values().flat_map(|s| s.missed.iter()).collect();
    if !missed.is_empty() {
        let _ = writeln!(report, "### Missed mutations\n");
        for m in missed {
            let _ = writeln!(report, "- {m}");
        }
    }
    match out_path {
        Some(p) => {
            std::fs::write(&p, &report).expect("write report");
            eprintln!("wrote {}", p.display());
        }
        None => print!("{report}"),
    }
}

impl Mutation {
    fn with_original(mut self, s: &str) -> Self {
        self.original = Some(s.to_string());
        self
    }
}

/// Abbreviates words of a snake_case name the way people often do (velocity → vel).
/// Returns None if nothing could be abbreviated.
fn abbreviate(s: &str) -> Option<String> {
    let mut changed = false;
    let words: Vec<String> = s
        .split('_')
        .map(|w| {
            let abbr = rosfix::assist::lexical::ABBREV
                .iter()
                .filter(|(a, full)| *full == w && a.len() >= 3 && a.len() < w.len())
                .map(|(a, _)| *a)
                .next(); // table order lists the most common abbreviation first
            match abbr {
                Some(a) => {
                    changed = true;
                    a.to_string()
                }
                None => w.to_string(),
            }
        })
        .collect();
    changed.then(|| words.join("_"))
}

/// Swaps two adjacent characters in the middle of `s` (a realistic typo).
fn typo(s: &str) -> Option<String> {
    let c: Vec<char> = s.chars().collect();
    if c.len() < 5 {
        return None;
    }
    let mid = c.len() / 2;
    for i in [mid, mid - 1, mid + 1] {
        if i + 1 < c.len()
            && c[i] != c[i + 1]
            && c[i].is_alphanumeric()
            && c[i + 1].is_alphanumeric()
        {
            let mut v = c.clone();
            v.swap(i, i + 1);
            return Some(v.into_iter().collect());
        }
    }
    None
}

/// Is `dep` reachable from `p`'s other exec/depend entries through workspace packages?
fn reachable_without(ix: &rosfix::workspace::Index, p: &rosfix::facts::Package, dep: &str) -> bool {
    use rosfix::facts::DepKind;
    let runtime = |k: DepKind| matches!(k, DepKind::Exec | DepKind::Depend);
    let mut seen = HashSet::new();
    let mut stack: Vec<&str> = p
        .deps
        .iter()
        .filter(|d| d.name != dep && runtime(d.kind))
        .map(|d| d.name.as_str())
        .collect();
    while let Some(n) = stack.pop() {
        if n == dep {
            return true;
        }
        if !seen.insert(n) {
            continue;
        }
        if let Some(q) = ix.package_by_name(n) {
            stack.extend(
                q.deps
                    .iter()
                    .filter(|d| runtime(d.kind))
                    .map(|d| d.name.as_str()),
            );
        }
    }
    false
}

fn replace_at(text: &str, start: usize, end: usize, with: &str) -> String {
    format!("{}{}{}", &text[..start], with, &text[end..])
}

fn discover(a: &Analysis, max: usize) -> Vec<Mutation> {
    let ix = &a.index;
    let mut out: Vec<Mutation> = Vec::new();
    let mut count: HashMap<&'static str, usize> = HashMap::new();
    let mut used: HashSet<(usize, usize)> = HashSet::new();
    let mut push = |m: Mutation, out: &mut Vec<Mutation>| {
        let c = count.entry(m.op).or_default();
        if *c < max {
            *c += 1;
            out.push(m);
        }
    };

    // ------------------------------------------------ parameter mutations (P003, P004)
    for (_, _, inst) in a.graph.instances() {
        if inst.impl_status != ImplStatus::Resolved || inst.params_open || inst.conditional {
            continue;
        }
        let decls = declared_params(ix, inst);
        let exact: HashSet<&str> = decls
            .iter()
            .filter_map(|d| match &d.name {
                NamePattern::Exact(n) => Some(n.as_str()),
                _ => None,
            })
            .collect();
        for b in &inst.params {
            let Some(fid) = b.from_file else { continue };
            if is_load_marker(b) || !used.insert((fid, b.key_loc.start)) {
                continue;
            }
            let f = ix.file(fid);
            let key = f.slice(b.key_loc);
            let Some(decl) = decls.iter().find(|d| d.name.matches(&b.name)) else {
                continue;
            };
            // P003: typo the key of a declared, top-level parameter.
            if key == b.name && matches!(decl.name, NamePattern::Exact(_)) {
                if let Some(t) = typo(key) {
                    if !exact.contains(t.as_str()) {
                        push(
                            Mutation {
                                op: "param-key-typo",
                                expect_rule: "P003",
                                file: f.abs_path.clone(),
                                rel: f.rel_path.clone(),
                                new_text: replace_at(&f.text, b.key_loc.start, b.key_loc.end, &t),
                                line: Some(f.line_col(b.key_loc.start).0),
                                message_has: None,
                                message_pkg: None,
                                original: None,

                                what: format!("`{key}` → `{t}` for {}", inst.display_name()),
                            }
                            .with_original(key),
                            &mut out,
                        );
                    }
                }
                // P003 (harder): the key written with common abbreviations (max_vel for
                // max_velocity) — not recoverable by edit distance alone.
                if let Some(ab) = abbreviate(key) {
                    if !exact.contains(ab.as_str()) {
                        push(
                            Mutation {
                                op: "param-key-abbrev",
                                expect_rule: "P003",
                                file: f.abs_path.clone(),
                                rel: f.rel_path.clone(),
                                new_text: replace_at(&f.text, b.key_loc.start, b.key_loc.end, &ab),
                                line: Some(f.line_col(b.key_loc.start).0),
                                message_has: None,
                                message_pkg: None,
                                original: Some(key.to_string()),
                                what: format!("`{key}` → `{ab}` for {}", inst.display_name()),
                            },
                            &mut out,
                        );
                    }
                }
            }
            // P004: write a declared double as an integer.
            if decl.ty == ParamType::Double {
                if let (Some(ParamValue::Double(v)), Some(raw)) = (&b.value, &b.raw) {
                    if v.fract() == 0.0 && raw.ends_with(".0") {
                        let int = raw.trim_end_matches(".0");
                        push(
                            Mutation {
                                op: "double-as-int",
                                expect_rule: "P004",
                                file: f.abs_path.clone(),
                                rel: f.rel_path.clone(),
                                new_text: replace_at(
                                    &f.text,
                                    b.value_loc.start,
                                    b.value_loc.end,
                                    int,
                                ),
                                line: Some(f.line_col(b.value_loc.start).0),
                                message_has: None,
                                message_pkg: None,
                                original: None,

                                what: format!(
                                    "`{}: {raw}` → `{int}` for {}",
                                    b.name,
                                    inst.display_name()
                                ),
                            },
                            &mut out,
                        );
                    }
                }
            }
        }
    }

    // ------------------------------------------------ P002: misspell ros__parameters
    let mut pfs: Vec<_> = ix.param_files.iter().collect();
    pfs.sort_by_key(|(k, _)| **k);
    for (fid, pf) in pfs {
        let f = ix.file(*fid);
        for s in &pf.sections {
            if let Some(off) = f.text[s.key_loc.end..].find("ros__parameters") {
                let start = s.key_loc.end + off;
                push(
                    Mutation {
                        op: "ros-parameters-typo",
                        expect_rule: "P002",
                        file: f.abs_path.clone(),
                        rel: f.rel_path.clone(),
                        new_text: replace_at(
                            &f.text,
                            start,
                            start + "ros__parameters".len(),
                            "ros_parameters",
                        ),
                        line: Some(f.line_col(start).0),
                        message_has: None,
                        message_pkg: None,
                        original: None,

                        what: format!("section `{}`", s.node_key),
                    },
                    &mut out,
                );
                break;
            }
        }
    }

    // ------------------------------------------------ L001: typo the executable in a launch file
    let mut seen_decl = HashSet::new();
    for (_, _, inst) in a.graph.instances() {
        let (Some(pkg), Some(exec)) = (&inst.pkg, &inst.exec) else {
            continue;
        };
        let Some(p) = ix.package_by_name(pkg) else {
            continue;
        };
        if !p.executables_complete
            || !p.executables.iter().any(|e| e.name == *exec)
            || !seen_decl.insert(inst.decl.loc)
        {
            continue;
        }
        let f = ix.file(inst.decl.loc.file);
        let span = &f.text[inst.decl.loc.start..inst.decl.loc.end.min(f.text.len())];
        // Target the executable attribute specifically (package and executable often share a name).
        let needles: Vec<String> = ["exec=", "executable="]
            .iter()
            .flat_map(|k| [format!("{k}\"{exec}\""), format!("{k}'{exec}'")])
            .collect();
        let Some((off, n)) = needles
            .iter()
            .find_map(|n| span.find(n.as_str()).map(|o| (o, n)))
        else {
            continue;
        };
        let Some(t) = typo(exec) else { continue };
        if p.executables.iter().any(|e| e.name == t) {
            continue;
        }
        let start = inst.decl.loc.start + off + n.find(['"', '\'']).unwrap_or(0) + 1;
        let end = start + exec.len();
        push(
            Mutation {
                op: "exec-name-typo",
                expect_rule: "L001",
                file: f.abs_path.clone(),
                rel: f.rel_path.clone(),
                new_text: replace_at(&f.text, start, end, &t),
                line: Some(f.line_col(inst.decl.loc.start).0),
                message_has: None,
                message_pkg: None,
                original: None,

                what: format!("`{exec}` → `{t}` (package {pkg})"),
            }
            .with_original(exec),
            &mut out,
        );
    }

    // ------------------------------------------------ L002: drop a declared runtime dependency used by launch
    for p in &ix.packages {
        let launch_pkgs: HashSet<String> = a
            .graph
            .instances()
            .filter(|(_, _, n)| {
                ix.package_of(n.decl.loc.file)
                    .is_some_and(|q| q.name == p.name)
            })
            .filter_map(|(_, _, n)| n.pkg.clone())
            .collect();
        let f = ix.file(p.manifest);
        for d in &p.deps {
            if d.name == p.name || !launch_pkgs.contains(&d.name) {
                continue;
            }
            // Only remove whole single-line entries.
            let line_start = f.text[..d.loc.start].rfind('\n').map_or(0, |i| i + 1);
            let line_end = f.text[d.loc.end..]
                .find('\n')
                .map_or(f.text.len(), |i| d.loc.end + i + 1);
            if f.text[line_start..d.loc.start].trim().is_empty()
                && f.text[d.loc.end..line_end].trim().is_empty()
            {
                // Skip if another entry still declares it, or another declared workspace
                // package already provides it transitively (rosfix accepts metapackages).
                if p.deps.iter().filter(|x| x.name == d.name).count() > 1
                    || reachable_without(ix, p, &d.name)
                {
                    continue;
                }
                push(
                    Mutation {
                        op: "drop-dependency",
                        expect_rule: "L002",
                        file: f.abs_path.clone(),
                        rel: f.rel_path.clone(),
                        new_text: replace_at(&f.text, line_start, line_end, ""),
                        line: None,
                        // Any L002 naming the dependency and package counts (the reason shown is
                        // that of its first use: find_package, import, or launch file).
                        message_has: Some(format!("`{}` is ", d.name)),
                        message_pkg: Some(format!("package `{}`", p.name)),
                        original: None,

                        what: format!("remove `{}` from {}", d.name, p.name),
                    },
                    &mut out,
                );
            }
        }
    }

    // ------------------------------------------------ N002: typo a Nav2 plugin type
    let mut pfs: Vec<_> = ix.param_files.iter().collect();
    pfs.sort_by_key(|(k, _)| **k);
    for (fid, pf) in &pfs {
        let f = ix.file(**fid);
        for s in &pf.sections {
            for e in &s.params {
                let ParamValue::Str(v) = &e.value else {
                    continue;
                };
                if !e.name.ends_with(".plugin")
                    || !rosfix::catalog::NAV2_PLUGINS.iter().any(|p| {
                        rosfix::catalog::normalize_plugin(p) == rosfix::catalog::normalize_plugin(v)
                    })
                {
                    continue;
                }
                let class = v.rsplit(['/', ':']).next().unwrap_or(v);
                let Some(t) = typo(class) else { continue };
                let new_v = format!("{}{}", &v[..v.len() - class.len()], t);
                let raw = f.slice(e.value_loc);
                let replaced = raw.replace(v.as_str(), &new_v);
                push(
                    Mutation {
                        op: "nav2-plugin-typo",
                        expect_rule: "N002",
                        file: f.abs_path.clone(),
                        rel: f.rel_path.clone(),
                        new_text: replace_at(
                            &f.text,
                            e.value_loc.start,
                            e.value_loc.end,
                            &replaced,
                        ),
                        line: Some(f.line_col(e.value_loc.start).0),
                        message_has: None,
                        message_pkg: None,
                        original: None,

                        what: format!("`{v}` → `{new_v}`"),
                    },
                    &mut out,
                );
            }
        }
    }

    // ------------------------------------------------ C001: typo a controller joint
    let control_joints: HashSet<&str> = ix
        .robots
        .iter()
        .flat_map(|r| {
            r.control
                .iter()
                .flat_map(|b| b.joints.iter().map(|j| j.name.as_str()))
        })
        .collect();
    if !control_joints.is_empty() && !ix.robots.iter().any(|r| r.partial) {
        for (fid, pf) in &pfs {
            let f = ix.file(**fid);
            for s in &pf.sections {
                for e in &s.params {
                    if !rosfix::catalog::is_joint_param(&e.name) {
                        continue;
                    }
                    let names = match &e.value {
                        ParamValue::StrArray(v) => v.clone(),
                        ParamValue::Str(v) => vec![v.clone()],
                        _ => continue,
                    };
                    let Some(j) = names.iter().find(|n| control_joints.contains(n.as_str())) else {
                        continue;
                    };
                    let Some(t) = typo(j) else { continue };
                    let raw = f.slice(e.value_loc);
                    let Some(off) = raw.find(j.as_str()) else {
                        continue;
                    };
                    let start = e.value_loc.start + off;
                    push(
                        Mutation {
                            op: "controller-joint-typo",
                            expect_rule: "C001",
                            file: f.abs_path.clone(),
                            rel: f.rel_path.clone(),
                            new_text: replace_at(&f.text, start, start + j.len(), &t),
                            line: Some(f.line_col(e.value_loc.start).0),
                            message_has: None,
                            message_pkg: None,
                            original: None,

                            what: format!("`{j}` → `{t}` in `{}`", e.name),
                        }
                        .with_original(j),
                        &mut out,
                    );
                }
            }
        }
    }
    let _ = Path::new(".");
    out
}
