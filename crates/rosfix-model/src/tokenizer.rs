//! Byte-level BPE. Ids 0..=255 are bytes, [`CLS`] = 256, merges start at [`FIRST_MERGE`].
//! Encoding is deterministic: at each step the merge with the lowest rank is applied to all
//! its occurrences, left to right.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

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
    ///
    /// Text is encoded word by word, with words cut into [`MAX_WORD`]-byte pieces exactly as
    /// in training, and encoding stops once `max_len` tokens are produced. A trained merge
    /// never spans a word boundary (whitespace only ever starts a word), so for words up to
    /// `MAX_WORD` bytes this equals applying the merges to the whole text; the cut keeps the
    /// cost linear in the text length.
    pub fn encode(&self, text: &str, max_len: usize) -> Vec<u32> {
        let cap = max_len.max(1);
        let mut v = vec![CLS];
        for w in split_words(text) {
            if v.len() >= cap {
                break;
            }
            for piece in w.as_bytes().chunks(MAX_WORD) {
                let room = cap.saturating_sub(v.len());
                if room == 0 {
                    break;
                }
                v.extend(self.encode_word(piece).into_iter().take(room));
            }
        }
        v
    }

    fn encode_word(&self, w: &[u8]) -> Vec<u32> {
        let mut ids: Vec<u32> = w.iter().map(|&b| u32::from(b)).collect();
        loop {
            let best = ids
                .windows(2)
                .filter_map(|p| self.rank.get(&(p[0], p[1])).map(|r| (*r, (p[0], p[1]))))
                .min();
            let Some((r, pair)) = best else { break };
            ids = merge_pair(&ids, pair, FIRST_MERGE + r);
        }
        ids
    }

    /// Learns `n_merges` merges from `texts`. Words (split at whitespace, kept with their
    /// leading space, cut into pieces of at most [`MAX_WORD`] bytes) are counted once each.
    /// Each step merges the most frequent pair (at least 2 occurrences); ties go to the
    /// smallest pair, so training is deterministic.
    ///
    /// Pair counts are kept up to date incrementally: a merge only re-counts the words that
    /// contain the merged pair, so the cost is proportional to the text a merge touches, not
    /// to the corpus size. The result is identical to recounting every pair at every step
    /// (tested against that reference below).
    pub fn train<'a>(texts: impl IntoIterator<Item = &'a str>, n_merges: usize) -> Self {
        let mut uniq: HashMap<Vec<u32>, u64> = HashMap::new();
        for t in texts {
            for w in split_words(t) {
                for piece in w.as_bytes().chunks(MAX_WORD) {
                    *uniq.entry(piece.iter().map(|&b| u32::from(b)).collect()).or_default() += 1;
                }
            }
        }
        let mut words: Vec<(Vec<u32>, u64)> = uniq.into_iter().collect();
        words.sort(); // deterministic order
        let mut counts: HashMap<(u32, u32), u64> = HashMap::new();
        let mut holders: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for (i, (w, n)) in words.iter().enumerate() {
            for p in w.windows(2) {
                let pair = (p[0], p[1]);
                *counts.entry(pair).or_default() += n;
                let h = holders.entry(pair).or_default();
                if h.last() != Some(&i) {
                    h.push(i);
                }
            }
        }
        // Max-heap of (count, smallest pair first); entries go stale when a count changes and
        // are skipped when popped.
        let mut heap: BinaryHeap<(u64, Reverse<(u32, u32)>)> =
            counts.iter().map(|(p, c)| (*c, Reverse(*p))).collect();
        let mut merges = Vec::new();
        while merges.len() < n_merges {
            let Some((c, Reverse(pair))) = heap.pop() else {
                break;
            };
            if counts.get(&pair) != Some(&c) {
                continue; // stale
            }
            if c < 2 {
                break;
            }
            let id = FIRST_MERGE + merges.len() as u32;
            merges.push(pair);
            let mut changed: Vec<(u32, u32)> = Vec::new();
            let mut hs = holders.remove(&pair).unwrap_or_default();
            hs.dedup();
            for wi in hs {
                let (w, n) = &mut words[wi];
                if !w.windows(2).any(|p| (p[0], p[1]) == pair) {
                    continue; // an earlier merge already consumed this occurrence
                }
                for p in w.windows(2) {
                    let k = (p[0], p[1]);
                    let e = counts.get_mut(&k).expect("counted pair");
                    *e -= *n;
                    changed.push(k);
                }
                let merged = merge_pair(w, pair, id);
                for p in merged.windows(2) {
                    let k = (p[0], p[1]);
                    *counts.entry(k).or_default() += *n;
                    changed.push(k);
                    if k.0 == id || k.1 == id {
                        let h = holders.entry(k).or_default();
                        if h.last() != Some(&wi) {
                            h.push(wi);
                        }
                    }
                }
                *w = merged;
            }
            counts.remove(&pair);
            changed.sort_unstable();
            changed.dedup();
            for k in changed {
                match counts.get(&k) {
                    Some(0) => {
                        counts.remove(&k);
                    }
                    Some(&c) => heap.push((c, Reverse(k))),
                    None => {}
                }
            }
        }
        Tokenizer::new(merges)
    }

    /// The merges as text, one `left right` pair of ids per line.
    pub fn to_text(&self) -> String {
        self.merges.iter().map(|(a, b)| format!("{a} {b}\n")).collect()
    }

    /// Parses [`Tokenizer::to_text`]; every id must be a byte or an earlier merge.
    pub fn from_text(s: &str) -> Result<Self, String> {
        let mut merges = Vec::new();
        for (i, line) in s.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
            let mut it = line.split_whitespace().map(str::parse::<u32>);
            let (Some(Ok(a)), Some(Ok(b)), None) = (it.next(), it.next(), it.next()) else {
                return Err(format!("line {}: expected two ids", i + 1));
            };
            let limit = FIRST_MERGE + merges.len() as u32;
            if a == CLS || b == CLS || a >= limit || b >= limit {
                return Err(format!("line {}: id out of range", i + 1));
            }
            merges.push((a, b));
        }
        Ok(Tokenizer::new(merges))
    }
}

/// Words longer than this are cut into pieces for training (long base64 blobs, URLs).
pub const MAX_WORD: usize = 48;

fn merge_pair(w: &[u32], pair: (u32, u32), id: u32) -> Vec<u32> {
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
    out
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

    /// The obvious algorithm: recount every pair before every merge.
    fn train_reference(texts: &[String], n_merges: usize) -> Vec<(u32, u32)> {
        let mut uniq: HashMap<Vec<u32>, u64> = HashMap::new();
        for t in texts {
            for w in split_words(t) {
                for piece in w.as_bytes().chunks(MAX_WORD) {
                    *uniq.entry(piece.iter().map(|&b| u32::from(b)).collect()).or_default() += 1;
                }
            }
        }
        let mut words: Vec<(Vec<u32>, u64)> = uniq.into_iter().collect();
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
            for (w, _) in &mut words {
                *w = merge_pair(w, pair, FIRST_MERGE + m as u32);
            }
            merges.push(pair);
        }
        merges
    }

    #[test]
    fn incremental_training_equals_the_reference() {
        // Random texts over a small alphabet, so pairs overlap ("aaa"), repeat and tie a lot.
        let mut s = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for round in 0..12 {
            let texts: Vec<String> = (0..40)
                .map(|_| {
                    let len = (next() % 120) as usize;
                    (0..len)
                        .map(|_| b"aab_: \n.ss"[(next() % 10) as usize] as char)
                        .collect()
                })
                .collect();
            let n = 10 + round * 25;
            let got = Tokenizer::train(texts.iter().map(String::as_str), n);
            assert_eq!(got.merges, train_reference(&texts, n), "round {round}");
        }
    }

    #[test]
    fn word_by_word_encoding_equals_whole_text_encoding() {
        let corpus = [
            "controller_server:\n  ros__parameters:\n    use_sim_time: true\n    controller_frequency: 20.0",
            "Node(package='nav2_controller', executable='controller_server', parameters=[p])",
            "<joint name=\"wheel_left_joint\" type=\"continuous\">  <axis xyz=\"0 0 1\"/>",
        ];
        let tok = Tokenizer::train(corpus.iter().copied(), 200);
        let whole = |text: &str| {
            // Lowest-rank merge applied to the whole byte sequence until none applies.
            let mut ids: Vec<u32> = text.bytes().map(u32::from).collect();
            while let Some((r, pair)) = ids
                .windows(2)
                .filter_map(|p| tok.rank.get(&(p[0], p[1])).map(|r| (*r, (p[0], p[1]))))
                .min()
            {
                ids = merge_pair(&ids, pair, FIRST_MERGE + r);
            }
            ids
        };
        for text in corpus.iter().copied().chain([
            "  use_sim_tme:   tru\n\tcontroler_frequency: 20",
            "executable='controller_srv'",
        ]) {
            let mut want = vec![CLS];
            want.extend(whole(text));
            assert_eq!(tok.encode(text, usize::MAX), want, "{text:?}");
            assert_eq!(tok.encode(text, 7), want[..7.min(want.len())], "{text:?}");
        }
    }

    #[test]
    fn merges_round_trip_as_text() {
        let corpus = ["use_sim_time: true", "use_sim_time: false", "base_frame_id: base"];
        let tok = Tokenizer::train(corpus.iter().copied(), 30);
        assert_eq!(Tokenizer::from_text(&tok.to_text()).unwrap(), tok);
        assert!(Tokenizer::from_text("97 98\n300 1\n").is_err(), "id from the future");
        assert!(Tokenizer::from_text("97\n").is_err());
    }

    #[test]
    fn truncates_and_keeps_bytes_without_merges() {
        let tok = Tokenizer::new(Vec::new());
        assert_eq!(tok.encode("ab", 10), vec![CLS, 97, 98]);
        assert_eq!(tok.encode("abcdef", 3), vec![CLS, 97, 98]);
        assert_eq!(tok.encode("", 3), vec![CLS]);
    }
}
