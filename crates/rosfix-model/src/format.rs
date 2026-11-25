//! The `.rfm` model file: one little-endian file, sections in fixed order, SHA-256 trailer.
//!
//! ```text
//! "RFM1" u16:format | str:tier | str:meta(JSON) | config 6×u32 (vocab d layers heads ffn ctx)
//! tokenizer: u32 n, n×(u32,u32) | emb Q8 | pos Q8 | layers × {norm, qkv Q8, f32s, out Q8, f32s,
//! norm, ff1 Q8, f32s, ff2 Q8, f32s} | ln_f norm | u32 heads × {str, f32s, f32} | sha256(all before)
//! ```
//! Q8 = u32 rows, u32 cols, rows·cols i8, rows f32 scales. Loading checks the trailer and every
//! tensor shape against the config, so a damaged or mismatched file is rejected, never trusted.

use crate::net::{Config, Head, Layer, Model, Norm, Q8};
use crate::sha256;
use crate::tokenizer::Tokenizer;

pub const MAGIC: &[u8; 4] = b"RFM1";
pub const FORMAT_VERSION: u16 = 1;
/// Refuse files larger than this before reading them (the largest small tier is ~25 MB).
pub const MAX_FILE_BYTES: u64 = 256 << 20;

pub fn to_bytes(m: &Model) -> Vec<u8> {
    let mut w = Vec::new();
    w.extend_from_slice(MAGIC);
    w.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    put_str(&mut w, &m.tier);
    put_str(&mut w, &m.meta);
    let c = m.config;
    for v in [c.vocab, c.d, c.layers, c.heads, c.ffn, c.ctx] {
        put_u32(&mut w, v);
    }
    put_u32(&mut w, m.tokenizer.merges.len() as u32);
    for (a, b) in &m.tokenizer.merges {
        put_u32(&mut w, *a);
        put_u32(&mut w, *b);
    }
    put_q8(&mut w, &m.emb);
    put_q8(&mut w, &m.pos);
    for l in &m.layers {
        put_norm(&mut w, &l.ln1);
        put_q8(&mut w, &l.qkv);
        put_f32s(&mut w, &l.qkv_b);
        put_q8(&mut w, &l.out);
        put_f32s(&mut w, &l.out_b);
        put_norm(&mut w, &l.ln2);
        put_q8(&mut w, &l.ff1);
        put_f32s(&mut w, &l.ff1_b);
        put_q8(&mut w, &l.ff2);
        put_f32s(&mut w, &l.ff2_b);
    }
    put_norm(&mut w, &m.ln_f);
    put_u32(&mut w, m.heads.len() as u32);
    for h in &m.heads {
        put_str(&mut w, &h.name);
        put_f32s(&mut w, &h.w);
        w.extend_from_slice(&h.b.to_le_bytes());
    }
    let digest = sha256::digest(&w);
    w.extend_from_slice(&digest);
    w
}

/// Parses and validates a model file. Returns the model and the file's SHA-256 (hex).
pub fn from_bytes(bytes: &[u8]) -> Result<(Model, String), String> {
    if bytes.len() < 4 + 2 + 32 || &bytes[..4] != MAGIC {
        return Err("not a rosfix model file (bad magic)".into());
    }
    let (body, trailer) = bytes.split_at(bytes.len() - 32);
    if sha256::digest(body) != trailer {
        return Err("model file is damaged (checksum mismatch)".into());
    }
    let mut r = Reader { b: body, i: 4 };
    let version = r.u16()?;
    if version != FORMAT_VERSION {
        return Err(format!(
            "unsupported model format {version} (this rosfix reads {FORMAT_VERSION})"
        ));
    }
    let tier = r.str()?;
    let meta = r.str()?;
    let config = Config {
        vocab: r.u32()?,
        d: r.u32()?,
        layers: r.u32()?,
        heads: r.u32()?,
        ffn: r.u32()?,
        ctx: r.u32()?,
    };
    config.validate()?;
    let n_merges = r.u32()? as usize;
    if n_merges as u64 + 257 > config.vocab as u64 {
        return Err("tokenizer is larger than the model vocabulary".into());
    }
    let mut merges = Vec::with_capacity(n_merges);
    for _ in 0..n_merges {
        merges.push((r.u32()?, r.u32()?));
    }
    let (d, f) = (config.d as usize, config.ffn as usize);
    let emb = r.q8(config.vocab as usize, d)?;
    let pos = r.q8(config.ctx as usize, d)?;
    let mut layers = Vec::with_capacity(config.layers as usize);
    for _ in 0..config.layers {
        layers.push(Layer {
            ln1: r.norm(d)?,
            qkv: r.q8(3 * d, d)?,
            qkv_b: r.f32s(3 * d)?,
            out: r.q8(d, d)?,
            out_b: r.f32s(d)?,
            ln2: r.norm(d)?,
            ff1: r.q8(f, d)?,
            ff1_b: r.f32s(f)?,
            ff2: r.q8(d, f)?,
            ff2_b: r.f32s(d)?,
        });
    }
    let ln_f = r.norm(d)?;
    let n_heads = r.u32()? as usize;
    let mut heads = Vec::with_capacity(n_heads.min(64));
    for _ in 0..n_heads {
        heads.push(Head {
            name: r.str()?,
            w: r.f32s(d)?,
            b: r.f32()?,
        });
    }
    if r.i != body.len() {
        return Err("model file has trailing data".into());
    }
    let model = Model {
        tier,
        config,
        tokenizer: Tokenizer::new(merges),
        emb,
        pos,
        layers,
        ln_f,
        heads,
        meta,
    };
    Ok((model, sha256::hex(trailer)))
}

pub fn load(path: &std::path::Path) -> Result<(Model, String), String> {
    let len = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    if len > MAX_FILE_BYTES {
        return Err(format!(
            "{}: {len} bytes is larger than any rosfix model",
            path.display()
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    from_bytes(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn save(path: &std::path::Path, m: &Model) -> std::io::Result<String> {
    let bytes = to_bytes(m);
    std::fs::write(path, &bytes)?;
    Ok(sha256::hex(&bytes[bytes.len() - 32..]))
}

fn put_u32(w: &mut Vec<u8>, v: u32) {
    w.extend_from_slice(&v.to_le_bytes());
}
fn put_str(w: &mut Vec<u8>, s: &str) {
    put_u32(w, s.len() as u32);
    w.extend_from_slice(s.as_bytes());
}
fn put_f32s(w: &mut Vec<u8>, v: &[f32]) {
    put_u32(w, v.len() as u32);
    for x in v {
        w.extend_from_slice(&x.to_le_bytes());
    }
}
fn put_norm(w: &mut Vec<u8>, n: &Norm) {
    put_f32s(w, &n.gain);
    put_f32s(w, &n.bias);
}
fn put_q8(w: &mut Vec<u8>, q: &Q8) {
    put_u32(w, q.rows as u32);
    put_u32(w, q.cols as u32);
    w.extend(q.data.iter().map(|v| *v as u8));
    for s in &q.scale {
        w.extend_from_slice(&s.to_le_bytes());
    }
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self
            .i
            .checked_add(n)
            .filter(|e| *e <= self.b.len())
            .ok_or("model file is truncated")?;
        let s = &self.b[self.i..end];
        self.i = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        let v = f32::from_le_bytes(self.take(4)?.try_into().unwrap());
        v.is_finite()
            .then_some(v)
            .ok_or_else(|| "model file contains a non-finite number".into())
    }
    fn str(&mut self) -> Result<String, String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec())
            .map_err(|_| "model file contains invalid text".into())
    }
    fn f32s(&mut self, expect: usize) -> Result<Vec<f32>, String> {
        let n = self.u32()? as usize;
        if n != expect {
            return Err(format!("tensor has {n} values, expected {expect}"));
        }
        (0..n).map(|_| self.f32()).collect()
    }
    fn norm(&mut self, d: usize) -> Result<Norm, String> {
        Ok(Norm {
            gain: self.f32s(d)?,
            bias: self.f32s(d)?,
        })
    }
    fn q8(&mut self, rows: usize, cols: usize) -> Result<Q8, String> {
        let (r, c) = (self.u32()? as usize, self.u32()? as usize);
        if (r, c) != (rows, cols) {
            return Err(format!("matrix is {r}x{c}, expected {rows}x{cols}"));
        }
        let data = self.take(r * c)?.iter().map(|v| *v as i8).collect();
        let scale = (0..r).map(|_| self.f32()).collect::<Result<_, _>>()?;
        Ok(Q8 {
            rows,
            cols,
            data,
            scale,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{net, Tier};

    fn tiny() -> Model {
        let c = Config {
            vocab: 300,
            d: 16,
            layers: 2,
            heads: 2,
            ffn: 32,
            ctx: 24,
        };
        let tok = Tokenizer::train(["use_sim_time: true", "use_sim_time"].iter().copied(), 10);
        net::random("test", c, tok, &["rank", "confidence"], 42)
    }

    #[test]
    fn roundtrip_is_exact_and_scores_identical() {
        let m = tiny();
        let bytes = to_bytes(&m);
        let (back, sha) = from_bytes(&bytes).unwrap();
        assert_eq!(back, m);
        assert_eq!(sha.len(), 64);
        let s1 = m
            .score("rank", "[CUR] max_velocty [CAND] max_velocity")
            .unwrap();
        let s2 = back
            .score("rank", "[CUR] max_velocty [CAND] max_velocity")
            .unwrap();
        assert_eq!(s1.to_bits(), s2.to_bits());
        assert!(s1.is_finite());
        assert!(back.score("missing-head", "x").is_none());
    }

    #[test]
    fn rejects_damaged_or_foreign_files() {
        let mut bytes = to_bytes(&tiny());
        assert!(from_bytes(b"not a model at all, definitely not").is_err());
        let mid = bytes.len() / 2;
        bytes[mid] ^= 1;
        assert!(from_bytes(&bytes).unwrap_err().contains("checksum"));
        assert!(from_bytes(&bytes[..bytes.len() - 40]).is_err());
    }

    #[test]
    fn tier_shapes_match_the_design() {
        // Sizes from the tier plan: nano ~1.5M, mini ~5.3M, base ~23M stored numbers.
        let p = |t: Tier| t.config().params() as f64 / 1e6;
        assert!(
            (1.3..1.7).contains(&p(Tier::Nano)),
            "nano {}",
            p(Tier::Nano)
        );
        assert!(
            (5.0..5.8).contains(&p(Tier::Mini)),
            "mini {}",
            p(Tier::Mini)
        );
        assert!(
            (22.0..25.0).contains(&p(Tier::Base)),
            "base {}",
            p(Tier::Base)
        );
        for t in [Tier::Nano, Tier::Mini, Tier::Base] {
            t.config().validate().unwrap();
        }
    }
}
