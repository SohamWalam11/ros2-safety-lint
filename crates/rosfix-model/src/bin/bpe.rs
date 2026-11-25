//! rosfix-model-bpe: learns the tier tokenizer from ROS 2 configuration files.
//!
//! Every directory given is one repository. Files are the ones rosfix reads (YAML, launch,
//! URDF/xacro/SRDF, package.xml), at most 256 KiB each, in sorted path order, so the result
//! depends only on the input files. One repository in ten (by name hash) is held out and
//! used to report compression (bytes per token) on unseen repositories.
//!
//! Usage: rosfix-model-bpe [--merges N] [--out merges.txt] REPO_DIR... | --repos-in DIR

use rosfix_model::tokenizer::{Tokenizer, FIRST_MERGE};
use std::path::{Path, PathBuf};
use std::time::Instant;

const MAX_FILE: u64 = 256 * 1024;

fn wanted(p: &Path) -> bool {
    let name = p
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    [".yaml", ".yml", ".launch.py", ".launch.xml", ".launch", ".urdf", ".xacro", ".srdf"]
        .iter()
        .any(|e| name.ends_with(e))
        || name == "package.xml"
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if e.file_name() != ".git" {
                walk(&p, out);
            }
        } else if ft.is_file()
            && wanted(&p)
            && e.metadata().map(|m| m.len() <= MAX_FILE).unwrap_or(false)
        {
            out.push(p);
        }
    }
}

fn fnv1a(s: &[u8]) -> u64 {
    s.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn main() {
    let mut merges = 8192 - FIRST_MERGE as usize;
    let mut out = PathBuf::from("merges.txt");
    let mut repos: Vec<PathBuf> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--merges" => merges = args.next().and_then(|v| v.parse().ok()).unwrap_or(merges),
            "--out" => out = args.next().map(PathBuf::from).unwrap_or(out),
            "--repos-in" => {
                let d = PathBuf::from(args.next().unwrap_or_default());
                let mut v: Vec<PathBuf> = std::fs::read_dir(&d)
                    .unwrap_or_else(|e| panic!("{}: {e}", d.display()))
                    .flatten()
                    .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                    .map(|e| e.path())
                    .collect();
                v.sort();
                repos.extend(v);
            }
            _ if !a.starts_with("--") => repos.push(PathBuf::from(a)),
            _ => {
                eprintln!("usage: rosfix-model-bpe [--merges N] [--out merges.txt] REPO_DIR... | --repos-in DIR");
                std::process::exit(2);
            }
        }
    }
    let mut train: Vec<String> = Vec::new();
    let mut test: Vec<String> = Vec::new();
    let (mut n_train_repos, mut n_test_repos) = (0, 0);
    for r in &repos {
        let name = r.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let held_out = fnv1a(name.as_bytes()).is_multiple_of(10);
        let mut files = Vec::new();
        walk(r, &mut files);
        let texts = files
            .iter()
            .filter_map(|f| std::fs::read(f).ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        if held_out {
            n_test_repos += 1;
            test.extend(texts);
        } else {
            n_train_repos += 1;
            train.extend(texts);
        }
    }
    let bytes = |v: &[String]| v.iter().map(String::len).sum::<usize>();
    eprintln!(
        "training on {} files ({:.1} MB) from {n_train_repos} repositories; {} files ({:.1} MB) from {n_test_repos} held out",
        train.len(),
        bytes(&train) as f64 / 1e6,
        test.len(),
        bytes(&test) as f64 / 1e6
    );
    let t0 = Instant::now();
    let tok = Tokenizer::train(train.iter().map(String::as_str), merges);
    let secs = t0.elapsed().as_secs_f64();
    std::fs::write(&out, tok.to_text()).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    let digest = rosfix_model::sha256::hex(&rosfix_model::sha256::digest(tok.to_text().as_bytes()));
    // Compression on held-out repositories (no truncation; CLS not counted).
    let (mut b, mut t) = (0usize, 0usize);
    for s in &test {
        b += s.len();
        t += tok.encode(s, usize::MAX).len() - 1;
    }
    println!(
        "merges {} (vocab {}), trained in {secs:.1} s, held-out bytes/token {:.2}, sha256 {digest}, written {}",
        tok.merges.len(),
        tok.vocab_size(),
        b as f64 / t.max(1) as f64,
        out.display()
    );
}
