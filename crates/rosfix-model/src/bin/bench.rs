//! rosfix-model-bench: measures each tier's real cost on this machine (random weights of the
//! exact tier shape; speed and size do not depend on the weights' values).
//!
//! For every tier: stored numbers, file size, load time, and latency per scored query on a
//! full-length input (the context is filled, so this is the worst case per query). Single
//! thread, as rosfix runs.
//!
//! Usage: rosfix-model-bench [--queries N] [--tier nano|mini|base]

use rosfix_model::{format, net, tokenizer::FIRST_MERGE, Tier, Tokenizer};
use std::time::Instant;

const QUERY: &str = "[RULE] P003 [OWNER] nav2_controller/controller_server [CUR] max_velocty \
[CAND] max_velocity [CTX] controller_server:\n  ros__parameters:\n    use_sim_time: True\n    \
controller_frequency: 20.0\n    min_x_velocity_threshold: 0.001\n    FollowPath:\n      plugin: \
\"dwb_core::DWBLocalPlanner\"\n      max_velocty: 0.26\n      min_vel_x: 0.0\n      acc_lim_x: 2.5\n";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut queries = 50usize;
    // Input length in tokens (default: fill each tier's context, the worst case). With
    // random merges one byte is one token, so the text is cut to this many bytes.
    let mut tokens_cap: Option<usize> = None;
    let mut only: Option<Tier> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--queries" => queries = it.next().and_then(|v| v.parse().ok()).unwrap_or(queries),
            "--tokens" => tokens_cap = it.next().and_then(|v| v.parse().ok()),
            "--tier" => only = it.next().and_then(|v| Tier::parse(v)),
            _ => {
                eprintln!(
                    "usage: rosfix-model-bench [--queries N] [--tokens N] [--tier nano|mini|base]"
                );
                std::process::exit(2);
            }
        }
    }
    // A long input (repeated) so every tier fills its context window.
    let mut text = QUERY.repeat(4);
    if let Some(n) = tokens_cap {
        text.truncate(n.saturating_sub(1)); // ASCII text; [CLS] is the extra token
    }
    println!(
        "| tier | stored numbers | file | load | query p50 | query p95 | tokens | deterministic |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---|");
    for tier in Tier::ALL
        .into_iter()
        .filter(|t| only.is_none_or(|o| o == *t))
    {
        let c = tier.config();
        // Byte-pair merges up to the vocabulary size (values do not matter for speed).
        let merges = (0..(c.vocab - FIRST_MERGE))
            .map(|i| (i % 256, (i / 256) % 256))
            .collect();
        let m = net::random(
            tier.name(),
            c,
            Tokenizer::new(merges),
            &["rank", "facts", "edit", "confidence"],
            7,
        );
        let path = std::env::temp_dir().join(format!("rosfix-bench-{}.rfm", tier.name()));
        format::save(&path, &m).expect("write model");
        let size = std::fs::metadata(&path).map(|x| x.len()).unwrap_or(0);
        let t = Instant::now();
        let (m, _) = format::load(&path).expect("load model");
        let load = t.elapsed();
        let _ = std::fs::remove_file(&path);
        let tokens = m.tokenizer.encode(&text, c.ctx as usize).len();
        let mut times = Vec::with_capacity(queries);
        let mut first = None;
        let mut same = true;
        for _ in 0..queries {
            let t = Instant::now();
            let s = m.score("rank", &text).expect("head");
            times.push(t.elapsed().as_secs_f64() * 1e3);
            match first {
                None => first = Some(s.to_bits()),
                Some(b) => same &= b == s.to_bits(),
            }
        }
        times.sort_by(|a, b| a.total_cmp(b));
        let q = |p: f64| times[((times.len() - 1) as f64 * p) as usize];
        println!(
            "| {} | {:.2}M | {:.1} MB | {:.0} ms | {:.1} ms | {:.1} ms | {tokens} | {} |",
            tier.name(),
            m.params() as f64 / 1e6,
            size as f64 / 1e6,
            load.as_secs_f64() * 1e3,
            q(0.5),
            q(0.95),
            if same { "yes" } else { "NO" }
        );
    }
}
