//! rosfix-history: a benchmark of *real* bugs, mined from repository history.
//!
//! For every recent non-merge commit whose message looks like a fix and which only modifies
//! ROS configuration files (parameter YAML, launch files, package.xml, CMakeLists.txt,
//! setup.py/cfg, URDF/xacro, RViz), the repository is analysed at the parent commit, and
//! again with the commit's versions of the modified files supplied in memory (everything
//! else identical). Findings are compared by fingerprint (rule, file, message):
//!
//! - **detected**: a finding in a package the commit modified disappears with the fix (rosfix
//!   flagged the bug before it was fixed);
//! - **introduced**: a finding in a modified package appears with the fix (a precision signal:
//!   either the fix is incomplete or rosfix is wrong about the fixed version).
//!
//! Usage: rosfix-history <repo-with-history>... [--max-commits N] [--out report.md] [--jsonl cases.jsonl]
//! Repositories may be partial clones (`git clone --filter=blob:none`); the working tree is
//! checked out at each parent commit (do not use a repository with local changes).

use rosfix::config::Config;
use rosfix::engine::{analyze, fingerprint, Analysis, Options};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(repo).args(["-c", "core.longpaths=true"]).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn is_config(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    [".yaml", ".yml", ".launch.py", ".launch.xml", ".launch.yaml", ".urdf", ".xacro", ".rviz"].iter().any(|e| name.ends_with(e))
        || matches!(name, "package.xml" | "CMakeLists.txt" | "setup.py" | "setup.cfg")
        || (path.contains("/launch/") && (name.ends_with(".py") || name.ends_with(".xml")))
}

fn looks_like_fix(subject: &str) -> bool {
    let s = subject.to_lowercase();
    let fix = ["fix", "typo", "wrong", "missing", "correct", "rename", "remap", "namespace", "frame", "depend", "install", "param", "qos", "use_sim_time", "bug"];
    let skip = ["merge", "release", "bump", "changelog", "version", "revert"];
    fix.iter().any(|w| s.contains(w)) && !skip.iter().any(|w| s.contains(w))
}

struct Case {
    repo: String,
    commit: String,
    subject: String,
    files: Vec<String>,
    detected: Vec<(String, String)>,
    introduced: Vec<(String, String)>,
}

/// Findings in the packages the commit touched. (A fix often edits a different file than the
/// one the finding is reported on, e.g. a dependency added to package.xml for an import in a
/// launch file.)
fn keyed(a: &Analysis, files: &BTreeSet<String>) -> BTreeMap<String, (String, String)> {
    let pkgs: BTreeSet<usize> = files.iter().filter_map(|f| a.index.find_rel(f)).filter_map(|id| a.index.file_pkg.get(id).copied().flatten()).collect();
    a.diagnostics
        .iter()
        .filter(|d| files.contains(&a.index.file(d.loc.file).rel_path) || a.index.file_pkg.get(d.loc.file).copied().flatten().is_some_and(|p| pkgs.contains(&p)))
        .map(|d| {
            let f = a.index.file(d.loc.file);
            (fingerprint(&a.index, d), (d.rule.to_string(), format!("{}:{}: {}", f.rel_path, f.line_col(d.loc.start).0, d.message)))
        })
        .collect()
}

fn mine(repo: &Path, max: usize, config: &Config) -> Vec<Case> {
    let name = repo.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
    let Some(log) = git(repo, &["log", "--no-merges", &format!("-{max}"), "--format=%H%x09%P%x09%s"]) else {
        eprintln!("[{name}] git log failed");
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in log.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(h), Some(parents), Some(subject)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let parents: Vec<&str> = parents.split_whitespace().collect();
        if parents.len() != 1 || !looks_like_fix(subject) {
            continue;
        }
        let Some(status) = git(repo, &["diff-tree", "--no-commit-id", "-r", "--name-status", h]) else { continue };
        let changes: Vec<(&str, &str)> = status.lines().filter_map(|l| l.split_once('\t')).collect();
        // Only modifications of configuration files (added/deleted files change the file set).
        if changes.is_empty() || changes.len() > 5 || !changes.iter().all(|(s, p)| *s == "M" && is_config(p)) {
            continue;
        }
        if git(repo, &["checkout", "--quiet", "--force", "--detach", parents[0]]).is_none() {
            eprintln!("[{name}] cannot check out {}", &parents[0][..8]);
            continue;
        }
        let cache = Arc::new(rosfix::workspace::ExtractCache::default());
        let before = analyze(repo, config, &Options { cache: Some(cache.clone()), ..Default::default() });
        let mut overrides = HashMap::new();
        let files: BTreeSet<String> = changes.iter().map(|(_, p)| p.to_string()).collect();
        for p in &files {
            if let Some(text) = git(repo, &["show", &format!("{h}:{p}")]) {
                overrides.insert(repo.join(p.replace('/', std::path::MAIN_SEPARATOR_STR)), text);
            }
        }
        let after = analyze(repo, config, &Options { cache: Some(cache), previous: Some(Arc::new(before.snapshot())), overrides, ..Default::default() });
        let (b, a) = (keyed(&before, &files), keyed(&after, &files));
        let case = Case {
            repo: name.clone(),
            commit: h[..10].to_string(),
            subject: subject.to_string(),
            files: files.into_iter().collect(),
            detected: b.iter().filter(|(k, _)| !a.contains_key(*k)).map(|(_, v)| v.clone()).collect(),
            introduced: a.iter().filter(|(k, _)| !b.contains_key(*k)).map(|(_, v)| v.clone()).collect(),
        };
        eprintln!(
            "[{name}] {} {:<60} detected {} introduced {}",
            case.commit,
            subject.chars().take(60).collect::<String>(),
            case.detected.len(),
            case.introduced.len()
        );
        out.push(case);
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut repos = Vec::new();
    let mut max = 1500usize;
    let mut out_path: Option<PathBuf> = None;
    let mut jsonl: Option<PathBuf> = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--max-commits" => max = it.next().and_then(|v| v.parse().ok()).unwrap_or(max),
            "--out" => out_path = it.next().map(PathBuf::from),
            "--jsonl" => jsonl = it.next().map(PathBuf::from),
            _ => repos.push(PathBuf::from(a)),
        }
    }
    if repos.is_empty() {
        eprintln!("usage: rosfix-history <repo>... [--max-commits N] [--out report.md] [--jsonl cases.jsonl]");
        std::process::exit(2);
    }
    let config = Config::default();
    let mut cases = Vec::new();
    for r in &repos {
        let head = git(r, &["rev-parse", "HEAD"]).unwrap_or_default();
        cases.extend(mine(r, max, &config));
        if !head.trim().is_empty() {
            let _ = git(r, &["checkout", "--quiet", "--force", "--detach", head.trim()]);
        }
    }
    // Report.
    let mut md = String::from("# rosfix on real fix commits\n\n");
    let _ = writeln!(md, "Candidate commits: non-merge, message looks like a fix, only modifies ROS configuration files (≤ 5).\n");
    let _ = writeln!(md, "| repository | candidates | detected (finding gone after the fix) | with introduced findings |\n|---|---:|---:|---:|");
    let mut by_repo: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
    for c in &cases {
        let e = by_repo.entry(&c.repo).or_default();
        e.0 += 1;
        e.1 += usize::from(!c.detected.is_empty());
        e.2 += usize::from(!c.introduced.is_empty());
    }
    let (mut tn, mut td, mut ti) = (0, 0, 0);
    for (r, (n, d, i)) in &by_repo {
        let _ = writeln!(md, "| {r} | {n} | {d} | {i} |");
        tn += n;
        td += d;
        ti += i;
    }
    let _ = writeln!(md, "| **all** | **{tn}** | **{td}** | **{ti}** |\n");
    let _ = writeln!(md, "## Detected\n");
    for c in cases.iter().filter(|c| !c.detected.is_empty()) {
        let _ = writeln!(md, "- {} `{}` {}", c.repo, c.commit, c.subject);
        for (rule, what) in &c.detected {
            let _ = writeln!(md, "  - {rule}: {what}");
        }
    }
    let _ = writeln!(md, "\n## Findings introduced by the fix\n");
    for c in cases.iter().filter(|c| !c.introduced.is_empty()) {
        let _ = writeln!(md, "- {} `{}` {}", c.repo, c.commit, c.subject);
        for (rule, what) in &c.introduced {
            let _ = writeln!(md, "  - {rule}: {what}");
        }
    }
    match &out_path {
        Some(p) => std::fs::write(p, &md).expect("write report"),
        None => print!("{md}"),
    }
    if let Some(p) = jsonl {
        let mut w = std::io::BufWriter::new(std::fs::File::create(p).expect("create jsonl"));
        for c in &cases {
            let line = serde_json::json!({
                "repo": c.repo, "commit": c.commit, "subject": c.subject, "files": c.files,
                "detected": c.detected, "introduced": c.introduced,
            });
            let _ = writeln!(w, "{line}");
        }
    }
    eprintln!("{tn} candidate commits, {td} detected, {ti} with introduced findings");
}
