//! Byte-level BPE. Ids 0..=255 are bytes, [`CLS`] = 256, merges start at [`FIRST_MERGE`].
//! Encoding is deterministic: at each step the merge with the lowest rank is applied to all
//! its occurrences, left to right.

use std::collections::HashMap;

pub const CLS: u32 = 256;
pub const FIRST_MERGE: u32 = 257;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokenizer {
    /// Merge `i` joins `merges[i].0` and `merges[i].1` into id `FIRST_MERGE + i`.
    pub merges: Vec<(u32, u32)>,
    rank: HashMap<(u32, u32), u32>,
}

impl Tokenizer {
    pub fn new(merges: Vec<(u32, u32)>) -> Self {
        let rank = merges
            .iter()
            .enumerate()
            .map(|(i, p)| (*p, i as u32))
            .collect();
        Tokenizer { merges, rank }
    }

    pub fn vocab_size(&self) -> u32 {
        FIRST_MERGE + self.merges.len() as u32
    }

    /// Token ids for `text`: [`CLS`] followed by at most `max_len - 1` tokens.
    pub fn encode(&self, text: &str, max_len: usize) -> Vec<u32> {
        let mut ids: Vec<u32> = text.bytes().map(u32::from).collect();
        loop {
            let best = ids
                .windows(2)
                .filter_map(|w| self.rank.get(&(w[0], w[1])).map(|r| (*r, (w[0], w[1]))))
                .min();
            let Some((r, pair)) = best else { break };
            let id = FIRST_MERGE + r;
            let mut out = Vec::with_capacity(ids.len());
            let mut i = 0;
            while i < ids.len() {
                if i + 1 < ids.len() && (ids[i], ids[i + 1]) == pair {
                    out.push(id);
                    i += 2;
                } else {
                    out.push(ids[i]);
                    i += 1;
                }
            }
            ids = out;
        }
        let mut v = Vec::with_capacity(max_len.min(ids.len() + 1));
        v.push(CLS);
        v.extend(ids.into_iter().take(max_len.saturating_sub(1)));
        v
    }

    /// Learns `n_merges` merges from `texts`. Words (split at whitespace, kept with their
    /// leading space) are counted once each; ties go to the smallest pair, so training is
    /// deterministic.
    pub fn train<'a>(texts: impl IntoIterator<Item = &'a str>, n_merges: usize) -> Self {
        let mut words: HashMap<Vec<u32>, u64> = HashMap::new();
        for t in texts {
            for w in split_words(t) {
                *words.entry(w.bytes().map(u32::from).collect()).or_default() += 1;
            }
        }
        let mut words: Vec<(Vec<u32>, u64)> = words.into_iter().collect();
        words.sort(); // deterministic iteration order
        let mut merges = Vec::new();
        for m in 0..n_merges {
            let mut counts: HashMap<(u32, u32), u64> = HashMap::new();
            for (w, n) in &words {
                for p in w.windows(2) {
                    *counts.entry((p[0], p[1])).or_default() += n;
                }
            }
            let Some((pair, _)) = counts
                .into_iter()
                .filter(|(_, c)| *c >= 2)
                .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            else {
                break;
            };
            let id = FIRST_MERGE + m as u32;
            for (w, _) in &mut words {
                let mut out = Vec::with_capacity(w.len());
                let mut i = 0;
                while i < w.len() {
                    if i + 1 < w.len() && (w[i], w[i + 1]) == pair {
                        out.push(id);
                        i += 2;
                    } else {
                        out.push(w[i]);
                        i += 1;
                    }
                }
                *w = out;
            }
            merges.push(pair);
        }
        Tokenizer::new(merges)
    }
}

fn split_words(t: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in t.char_indices() {
        if c.is_whitespace() && i > start {
            out.push(&t[start..i]);
            start = i;
        }
    }
    if start < t.len() {
        out.push(&t[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_and_applies_merges() {
        let corpus = ["use_sim_time: true", "use_sim_time: false", "use_sim_time"];
        let tok = Tokenizer::train(corpus.iter().copied(), 20);
        assert!(!tok.merges.is_empty());
        let ids = tok.encode("use_sim_time", 64);
        assert_eq!(ids[0], CLS);
        assert!(
            ids.len() < "use_sim_time".len() + 1,
            "merges shorten the sequence"
        );
        // Training is deterministic.
        assert_eq!(tok, Tokenizer::train(corpus.iter().copied(), 20));
    }

    #[test]
    fn truncates_and_keeps_bytes_without_merges() {
        let tok = Tokenizer::new(Vec::new());
        assert_eq!(tok.encode("ab", 10), vec![CLS, 97, 98]);
        assert_eq!(tok.encode("abcdef", 3), vec![CLS, 97, 98]);
        assert_eq!(tok.encode("", 3), vec![CLS]);
    }
}
