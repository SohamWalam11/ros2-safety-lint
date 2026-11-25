//! The encoder: a pre-LayerNorm transformer with int8 weights.
//!
//! Determinism: weights and activations meet in int8 × int8 → i32 dot products, which are
//! exact, so any SIMD or loop order gives the same bits. Everything in f32 runs in a fixed
//! order, and `exp`/`tanh` are computed here from IEEE-exact operations (+, −, ×, ÷, sqrt)
//! instead of the platform libm, whose last bit differs between operating systems.

use crate::tokenizer::Tokenizer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub vocab: u32,
    pub d: u32,
    pub layers: u32,
    pub heads: u32,
    pub ffn: u32,
    pub ctx: u32,
}

impl Config {
    /// Number of stored numbers (weights, biases, LayerNorm gains) excluding heads.
    pub fn params(&self) -> u64 {
        let (v, d, f, c) = (
            self.vocab as u64,
            self.d as u64,
            self.ffn as u64,
            self.ctx as u64,
        );
        let layer = 2 * d + (3 * d * d + 3 * d) + (d * d + d) + 2 * d + (d * f + f) + (f * d + d);
        v * d + c * d + self.layers as u64 * layer + 2 * d
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.d == 0 || self.heads == 0 || !self.d.is_multiple_of(self.heads) {
            return Err(format!(
                "d ({}) must be a positive multiple of heads ({})",
                self.d, self.heads
            ));
        }
        if self.ctx < 2 || self.vocab < 257 || self.ffn == 0 || self.layers == 0 {
            return Err("invalid model shape".into());
        }
        Ok(())
    }
}

/// Row-wise symmetric int8 matrix: `w[r][c] = data[r * cols + c] * scale[r]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q8 {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<i8>,
    pub scale: Vec<f32>,
}

impl Q8 {
    /// `W x + b`. The input is quantised per call to int8 with one scale.
    pub fn matvec(&self, x: &[f32], bias: &[f32]) -> Vec<f32> {
        debug_assert_eq!(x.len(), self.cols);
        let amax = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        if amax == 0.0 {
            return bias.to_vec();
        }
        let sx = amax / 127.0;
        let q: Vec<i8> = x
            .iter()
            .map(|v| (v / sx).round().clamp(-127.0, 127.0) as i8)
            .collect();
        let dot = dot_fn();
        (0..self.rows)
            .map(|r| {
                let row = &self.data[r * self.cols..(r + 1) * self.cols];
                dot(row, &q) as f32 * (self.scale[r] * sx) + bias[r]
            })
            .collect()
    }
}

/// int8 dot product with an i32 result. Integer addition is exact and associative, so every
/// variant (scalar, AVX2) returns the same value: the choice never changes a model's output.
fn dot_scalar(a: &[i8], b: &[i8]) -> i32 {
    // 16 independent i32 lanes so the compiler can vectorise; |a·b| ≤ 127², and a row of
    // up to 2^17 columns cannot overflow i32.
    let n = a.len().min(b.len());
    let mut lanes = [0i32; 16];
    let (ca, ta) = a[..n].as_chunks::<16>();
    let (cb, tb) = b[..n].as_chunks::<16>();
    for (x, y) in ca.iter().zip(cb) {
        for k in 0..16 {
            lanes[k] += i32::from(x[k]) * i32::from(y[k]);
        }
    }
    let tail: i32 = ta
        .iter()
        .zip(tb)
        .map(|(x, y)| i32::from(*x) * i32::from(*y))
        .sum();
    lanes.iter().sum::<i32>() + tail
}

/// AVX2: sign-extend 16 int8 to int16, `vpmaddwd` multiplies pairs and adds them into 8 i32
/// lanes (exact: |2·127²| fits easily). The tail and the lane sum are plain integer code.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn dot_avx2(a: &[i8], b: &[i8]) -> i32 {
    use std::arch::x86_64::*;
    let n = a.len().min(b.len());
    let mut acc0 = _mm256_setzero_si256();
    let mut acc1 = _mm256_setzero_si256();
    let mut i = 0;
    while i + 32 <= n {
        let a0 = _mm256_cvtepi8_epi16(_mm_loadu_si128(a.as_ptr().add(i) as *const __m128i));
        let b0 = _mm256_cvtepi8_epi16(_mm_loadu_si128(b.as_ptr().add(i) as *const __m128i));
        let a1 = _mm256_cvtepi8_epi16(_mm_loadu_si128(a.as_ptr().add(i + 16) as *const __m128i));
        let b1 = _mm256_cvtepi8_epi16(_mm_loadu_si128(b.as_ptr().add(i + 16) as *const __m128i));
        acc0 = _mm256_add_epi32(acc0, _mm256_madd_epi16(a0, b0));
        acc1 = _mm256_add_epi32(acc1, _mm256_madd_epi16(a1, b1));
        i += 32;
    }
    let mut lanes = [0i32; 8];
    _mm256_storeu_si256(
        lanes.as_mut_ptr() as *mut __m256i,
        _mm256_add_epi32(acc0, acc1),
    );
    lanes.iter().sum::<i32>() + dot_scalar(&a[i..n], &b[i..n])
}

fn dot_fn() -> fn(&[i8], &[i8]) -> i32 {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            // SAFETY: only selected when the CPU reports AVX2.
            return |a, b| unsafe { dot_avx2(a, b) };
        }
    }
    dot_scalar
}

impl Q8 {
    /// `matvec` for many inputs at once, bit-identical to calling it per input: each input is
    /// quantised the same way and each output uses the same integer dot product and the same
    /// float formula. Only the loop order differs (rows outside, inputs inside), which keeps
    /// one weight row in cache while it meets every input.
    pub fn matmul(&self, xs: &[Vec<f32>], bias: &[f32]) -> Vec<Vec<f32>> {
        let quant: Vec<Option<(Vec<i8>, f32)>> = xs
            .iter()
            .map(|x| {
                debug_assert_eq!(x.len(), self.cols);
                let amax = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                (amax != 0.0).then(|| {
                    let sx = amax / 127.0;
                    let q = x
                        .iter()
                        .map(|v| (v / sx).round().clamp(-127.0, 127.0) as i8)
                        .collect();
                    (q, sx)
                })
            })
            .collect();
        let mut out: Vec<Vec<f32>> = vec![vec![0.0; self.rows]; xs.len()];
        let dot = dot_fn();
        for r in 0..self.rows {
            let row = &self.data[r * self.cols..(r + 1) * self.cols];
            for (t, q) in quant.iter().enumerate() {
                out[t][r] = match q {
                    Some((q, sx)) => dot(row, q) as f32 * (self.scale[r] * sx) + bias[r],
                    None => bias[r],
                };
            }
        }
        out
    }

    pub fn row(&self, r: usize) -> Vec<f32> {
        self.data[r * self.cols..(r + 1) * self.cols]
            .iter()
            .map(|v| f32::from(*v) * self.scale[r])
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Norm {
    pub gain: Vec<f32>,
    pub bias: Vec<f32>,
}

impl Norm {
    fn apply(&self, x: &[f32]) -> Vec<f32> {
        let n = x.len() as f32;
        let mean = x.iter().sum::<f32>() / n;
        let var = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n;
        let inv = 1.0 / (var + 1e-5).sqrt();
        x.iter()
            .zip(self.gain.iter().zip(&self.bias))
            .map(|(v, (g, b))| (v - mean) * inv * g + b)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub ln1: Norm,
    pub qkv: Q8,
    pub qkv_b: Vec<f32>,
    pub out: Q8,
    pub out_b: Vec<f32>,
    pub ln2: Norm,
    pub ff1: Q8,
    pub ff1_b: Vec<f32>,
    pub ff2: Q8,
    pub ff2_b: Vec<f32>,
}

/// A linear scoring head on the pooled ([CLS]) vector.
#[derive(Debug, Clone, PartialEq)]
pub struct Head {
    pub name: String,
    pub w: Vec<f32>,
    pub b: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub tier: String,
    pub config: Config,
    pub tokenizer: Tokenizer,
    pub emb: Q8,
    pub pos: Q8,
    pub layers: Vec<Layer>,
    pub ln_f: Norm,
    pub heads: Vec<Head>,
    /// Free-form JSON: training data snapshot, evaluation scores, licence notes.
    pub meta: String,
}

impl Model {
    /// The pooled representation of `text` (final LayerNorm of the [CLS] position).
    pub fn encode(&self, text: &str) -> Vec<f32> {
        let ids = self.tokenizer.encode(text, self.config.ctx as usize);
        let mut h: Vec<Vec<f32>> = ids
            .iter()
            .enumerate()
            .map(|(p, id)| {
                let tok = self.emb.row((*id).min(self.config.vocab - 1) as usize);
                let pos = self.pos.row(p);
                tok.iter().zip(&pos).map(|(a, b)| a + b).collect()
            })
            .collect();
        for l in &self.layers {
            h = self.layer(l, &h);
        }
        self.ln_f.apply(&h[0])
    }

    fn layer(&self, l: &Layer, h: &[Vec<f32>]) -> Vec<Vec<f32>> {
        let d = self.config.d as usize;
        let nh = self.config.heads as usize;
        let hd = d / nh;
        // Every projection runs on all positions at once (`matmul`), so each weight row is
        // read from memory once per query instead of once per token.
        let normed: Vec<Vec<f32>> = h.iter().map(|x| l.ln1.apply(x)).collect();
        let qkv = l.qkv.matmul(&normed, &l.qkv_b);
        let inv = 1.0 / (hd as f32).sqrt();
        let n = h.len();
        let mut ctxs = vec![vec![0.0f32; d]; n];
        let mut scores = vec![0.0f32; n];
        for head in 0..nh {
            // Contiguous per-head K and V (n × hd) so the inner loops stream memory.
            let k: Vec<f32> = qkv
                .iter()
                .flat_map(|t| t[d + head * hd..d + (head + 1) * hd].iter().copied())
                .collect();
            let v: Vec<f32> = qkv
                .iter()
                .flat_map(|t| {
                    t[2 * d + head * hd..2 * d + (head + 1) * hd]
                        .iter()
                        .copied()
                })
                .collect();
            let attend = attend_fn();
            for (i, ctx) in ctxs.iter_mut().enumerate() {
                let q = &qkv[i][head * hd..(head + 1) * hd];
                attend(
                    q,
                    &k,
                    &v,
                    hd,
                    inv,
                    &mut scores,
                    &mut ctx[head * hd..(head + 1) * hd],
                );
            }
        }
        let attn = l.out.matmul(&ctxs, &l.out_b);
        let x1: Vec<Vec<f32>> = h
            .iter()
            .zip(&attn)
            .map(|(x, a)| x.iter().zip(a).map(|(u, v)| u + v).collect())
            .collect();
        let normed: Vec<Vec<f32>> = x1.iter().map(|x| l.ln2.apply(x)).collect();
        let mut f = l.ff1.matmul(&normed, &l.ff1_b);
        for row in &mut f {
            for v in row.iter_mut() {
                *v = gelu(*v);
            }
        }
        let f = l.ff2.matmul(&f, &l.ff2_b);
        x1.iter()
            .zip(&f)
            .map(|(u, v)| u.iter().zip(v).map(|(a, b)| a + b).collect())
            .collect()
    }

    pub fn head(&self, name: &str) -> Option<&Head> {
        self.heads.iter().find(|h| h.name == name)
    }

    /// Score of `text` under head `name` (higher is better), or None if the head is absent.
    pub fn score(&self, name: &str, text: &str) -> Option<f32> {
        let head = self.head(name)?;
        let v = self.encode(text);
        Some(head.w.iter().zip(&v).map(|(a, b)| a * b).sum::<f32>() + head.b)
    }

    /// Number of stored numbers including heads.
    pub fn params(&self) -> u64 {
        self.config.params() + self.heads.iter().map(|h| h.w.len() as u64 + 1).sum::<u64>()
    }
}

/// e^x from IEEE-exact operations only: x = k·ln2 + r, |r| ≤ ln2/2, e^r by a degree-7
/// Taylor polynomial (relative error < 2e-7 on that range), times 2^k built from bits.
pub fn exp(x: f32) -> f32 {
    if x.is_nan() {
        return x;
    }
    let x = x.clamp(-87.0, 88.0);
    const LN2: f32 = std::f32::consts::LN_2;
    let k = (x / LN2).round();
    let r = x - k * LN2;
    let mut p = 1.0f32 / 5040.0;
    for c in [
        1.0f32 / 720.0,
        1.0 / 120.0,
        1.0 / 24.0,
        1.0 / 6.0,
        0.5,
        1.0,
        1.0,
    ] {
        p = p * r + c;
    }
    p * f32::from_bits(((k as i32 + 127) as u32) << 23)
}

pub fn tanh(x: f32) -> f32 {
    if x > 9.0 {
        return 1.0;
    }
    if x < -9.0 {
        return -1.0;
    }
    let e = exp(2.0 * x);
    (e - 1.0) / (e + 1.0)
}

pub(crate) fn gelu(x: f32) -> f32 {
    0.5 * x * (1.0 + tanh(0.797_884_6 * (x + 0.044_715 * x * x * x)))
}

fn softmax_in_place(x: &mut [f32]) {
    let m = x.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let mut s = 0.0f32;
    for v in x.iter_mut() {
        *v = exp(*v - m);
        s += *v;
    }
    for v in x.iter_mut() {
        *v /= s;
    }
}

type AttendFn = fn(&[f32], &[f32], &[f32], usize, f32, &mut [f32], &mut [f32]);

/// One query position, one head: scores against every key, softmax, weighted sum of values.
#[inline(always)]
fn attend(
    q: &[f32],
    k: &[f32],
    v: &[f32],
    hd: usize,
    inv: f32,
    scores: &mut [f32],
    out: &mut [f32],
) {
    for (j, s) in scores.iter_mut().enumerate() {
        *s = dot_f32(q, &k[j * hd..(j + 1) * hd]) * inv;
    }
    softmax_in_place(scores);
    for (j, p) in scores.iter().enumerate() {
        // Element-wise (no reduction): vectorisable without changing the result.
        for (c, vv) in out.iter_mut().zip(&v[j * hd..(j + 1) * hd]) {
            *c += p * vv;
        }
    }
}

fn attend_scalar(
    q: &[f32],
    k: &[f32],
    v: &[f32],
    hd: usize,
    inv: f32,
    s: &mut [f32],
    o: &mut [f32],
) {
    attend(q, k, v, hd, inv, s, o)
}

/// The same code with 256-bit registers. AVX2 implies no FMA here (Rust never contracts
/// `a * b + c`), so every operation and its order are the same as in `attend_scalar`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn attend_avx2(
    q: &[f32],
    k: &[f32],
    v: &[f32],
    hd: usize,
    inv: f32,
    s: &mut [f32],
    o: &mut [f32],
) {
    attend(q, k, v, hd, inv, s, o)
}

fn attend_fn() -> AttendFn {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            // SAFETY: only selected when the CPU reports AVX2.
            return |q, k, v, hd, inv, s, o| unsafe { attend_avx2(q, k, v, hd, inv, s, o) };
        }
    }
    attend_scalar
}

/// f32 dot product in a fixed order: 8 running lanes, then the lanes summed left to right.
/// The order is part of the model's definition, so results are identical on every CPU,
/// while the 8 independent lanes still let the compiler use SIMD.
pub(crate) fn dot_f32(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let mut lanes = [0.0f32; 8];
    let (ca, ta) = a[..n].as_chunks::<8>();
    let (cb, tb) = b[..n].as_chunks::<8>();
    for (x, y) in ca.iter().zip(cb) {
        for k in 0..8 {
            lanes[k] += x[k] * y[k];
        }
    }
    let mut s = 0.0f32;
    for l in lanes {
        s += l;
    }
    for (x, y) in ta.iter().zip(tb) {
        s += x * y;
    }
    s
}

/// A model with random weights of the given shape (tests and benchmarks only; never shipped).
pub fn random(
    tier: &str,
    config: Config,
    tokenizer: Tokenizer,
    heads: &[&str],
    seed: u64,
) -> Model {
    let mut s = seed | 1;
    let mut next = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let mut q8 = |rows: usize, cols: usize| Q8 {
        rows,
        cols,
        data: (0..rows * cols)
            .map(|_| ((next() % 255) as i32 - 127) as i8)
            .collect(),
        scale: vec![1.0 / (127.0 * (cols as f32).sqrt()); rows],
    };
    let (d, f, v, c) = (
        config.d as usize,
        config.ffn as usize,
        config.vocab as usize,
        config.ctx as usize,
    );
    let norm = || Norm {
        gain: vec![1.0; d],
        bias: vec![0.0; d],
    };
    let layers = (0..config.layers)
        .map(|_| Layer {
            ln1: norm(),
            qkv: q8(3 * d, d),
            qkv_b: vec![0.0; 3 * d],
            out: q8(d, d),
            out_b: vec![0.0; d],
            ln2: norm(),
            ff1: q8(f, d),
            ff1_b: vec![0.0; f],
            ff2: q8(d, f),
            ff2_b: vec![0.0; d],
        })
        .collect();
    let emb = q8(v, d);
    let pos = q8(c, d);
    Model {
        tier: tier.to_string(),
        config,
        tokenizer,
        emb,
        pos,
        layers,
        ln_f: norm(),
        heads: heads
            .iter()
            .enumerate()
            .map(|(i, n)| Head {
                name: n.to_string(),
                w: (0..d)
                    .map(|j| if (i + j) % 2 == 0 { 0.05 } else { -0.05 })
                    .collect(),
                b: 0.0,
            })
            .collect(),
        meta: "{\"random\": true}".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exp_matches_libm_closely() {
        // Softmax inputs are <= 0 after the max shift; GELU's tanh needs |2x| <= 18.
        for i in -80..=80 {
            let x = i as f32 * 0.25;
            let (a, b) = (exp(x), x.exp());
            assert!(((a - b) / b).abs() < 2e-6, "exp({x}) = {a} vs {b}");
        }
        assert!((tanh(0.5) - 0.5f32.tanh()).abs() < 1e-6);
    }

    #[test]
    fn every_dot_product_variant_gives_the_same_integer() {
        let mut s = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            ((s % 255) as i32 - 127) as i8
        };
        for len in [0usize, 1, 15, 16, 17, 255, 1024, 2051] {
            let a: Vec<i8> = (0..len).map(|_| next()).collect();
            let b: Vec<i8> = (0..len).map(|_| next()).collect();
            let naive: i32 = a
                .iter()
                .zip(&b)
                .map(|(x, y)| i32::from(*x) * i32::from(*y))
                .sum();
            assert_eq!(dot_scalar(&a, &b), naive);
            assert_eq!(dot_fn()(&a, &b), naive);
        }
    }

    /// Where a mini query's time goes (run: cargo test --release -p rosfix-model -- --ignored --nocapture).
    #[test]
    #[ignore]
    fn profile_mini_components() {
        use std::time::Instant;
        let c = crate::Tier::Mini.config();
        let (d, f, n) = (c.d as usize, c.ffn as usize, c.ctx as usize);
        let m = random("p", c, Tokenizer::new(Vec::new()), &["rank"], 3);
        let xs: Vec<Vec<f32>> = (0..n)
            .map(|t| (0..d).map(|i| ((t * d + i) as f32).sin()).collect())
            .collect();
        let l = &m.layers[0];
        let t = Instant::now();
        let _ = l.ff1.matmul(&xs, &vec![0.0; f]);
        println!(
            "ff1 matmul {n}x({f}x{d}): {:.1} ms",
            t.elapsed().as_secs_f64() * 1e3
        );
        let t = Instant::now();
        for x in &xs {
            let _ = l.ff1.matvec(x, &vec![0.0; f]);
        }
        println!(
            "ff1 per-token matvec: {:.1} ms",
            t.elapsed().as_secs_f64() * 1e3
        );
        let a: Vec<i8> = (0..d).map(|i| (i % 100) as i8).collect();
        let t = Instant::now();
        let mut s = 0i64;
        for _ in 0..(n * f) {
            s += i64::from(dot_fn()(&a, &a));
        }
        println!(
            "{} dots of {d}: {:.1} ms (sum {s})",
            n * f,
            t.elapsed().as_secs_f64() * 1e3
        );
        let t = Instant::now();
        let _ = m.layer(l, &xs);
        println!("one full layer: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        let t = Instant::now();
        let mut e = 0.0f32;
        for i in 0..(n * n * c.heads as usize) {
            e += exp(-((i % 50) as f32) * 0.1);
        }
        println!(
            "{} exp calls (one layer's softmax): {:.1} ms ({e})",
            n * n * c.heads as usize,
            t.elapsed().as_secs_f64() * 1e3
        );
    }

    #[test]
    fn attention_variants_are_bit_identical() {
        let (n, hd) = (37usize, 24usize);
        let f = |i: usize| ((i as f32) * 0.731).sin();
        let q: Vec<f32> = (0..hd).map(f).collect();
        let k: Vec<f32> = (0..n * hd).map(|i| f(i + 1000)).collect();
        let v: Vec<f32> = (0..n * hd).map(|i| f(i + 5000)).collect();
        let run = |g: AttendFn| {
            let (mut s, mut o) = (vec![0.0; n], vec![0.25; hd]);
            g(&q, &k, &v, hd, 0.2, &mut s, &mut o);
            o.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
        };
        assert_eq!(run(attend_scalar), run(attend_fn()));
    }

    #[test]
    fn batched_matmul_is_bit_identical_to_matvec() {
        let m = Q8 {
            rows: 5,
            cols: 33,
            data: (0..165i32).map(|i| (i * 37 % 255 - 127) as i8).collect(),
            scale: (0..5).map(|r| 0.001 * (r + 1) as f32).collect(),
        };
        let bias: Vec<f32> = (0..5).map(|r| r as f32 * 0.1).collect();
        let xs: Vec<Vec<f32>> = (0..4)
            .map(|t| {
                (0..33)
                    .map(|c| ((t * 33 + c) as f32 * 0.37).sin() * (t as f32))
                    .collect()
            })
            .collect(); // xs[0] is all zeros: the bias-only path
        let batched = m.matmul(&xs, &bias);
        for (x, y) in xs.iter().zip(&batched) {
            let single = m.matvec(x, &bias);
            assert_eq!(
                single.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                y.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn matvec_is_close_to_float_math() {
        let m = Q8 {
            rows: 2,
            cols: 3,
            data: vec![127, 0, -127, 64, 64, 64],
            scale: vec![0.01, 0.02],
        };
        let y = m.matvec(&[1.0, 2.0, 3.0], &[0.5, 0.0]);
        let exact = [1.27 - 3.81 + 0.5, 1.28 * 6.0];
        for (a, b) in y.iter().zip(exact) {
            assert!((a - b).abs() < 0.02, "{a} vs {b}");
        }
    }
}
