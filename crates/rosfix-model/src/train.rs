//! Training for the tier models, in plain Rust (no ML framework).
//!
//! [`TrainModel`] is the f32 twin of the runtime [`crate::Model`]: same pre-LayerNorm
//! encoder, same GELU and attention, [CLS] pooling, linear heads. It adds the backward pass
//! (written out by hand, checked against finite differences in the tests), Adam, and the
//! listwise ranking loss rosfix's assistant uses at runtime: a softmax over the candidates'
//! scores plus a fixed "none of these" score of 0, so the model learns when to abstain.
//! Export to the int8 runtime format is in [`TrainModel::export`].
//!
//! Training is seeded. A step runs on a fixed number of threads ([`SHARDS`]) whose gradients
//! are added in a fixed order, and all floating-point code runs in a fixed order without fused
//! multiply-add, so a seed gives the same model on every machine.

use crate::net::{dot_f32, exp, gelu, tanh, Config};

/// One trainable tensor with its gradient and Adam moments.
#[derive(Debug, Clone)]
pub struct Param {
    pub w: Vec<f32>,
    pub g: Vec<f32>,
    m: Vec<f32>,
    v: Vec<f32>,
}

impl Param {
    fn new(w: Vec<f32>) -> Self {
        let n = w.len();
        Param {
            w,
            g: vec![0.0; n],
            m: vec![0.0; n],
            v: vec![0.0; n],
        }
    }
}

#[derive(Debug, Clone)]
pub struct LayerParams {
    pub ln1_g: Param,
    pub ln1_b: Param,
    pub qkv_w: Param,
    pub qkv_b: Param,
    pub out_w: Param,
    pub out_b: Param,
    pub ln2_g: Param,
    pub ln2_b: Param,
    pub ff1_w: Param,
    pub ff1_b: Param,
    pub ff2_w: Param,
    pub ff2_b: Param,
}

#[derive(Debug, Clone)]
pub struct HeadParams {
    pub name: String,
    pub w: Param,
    pub b: Param,
}

#[derive(Debug, Clone)]
pub struct TrainModel {
    pub tier: String,
    pub config: Config,
    pub emb: Param,
    pub pos: Param,
    pub layers: Vec<LayerParams>,
    pub lnf_g: Param,
    pub lnf_b: Param,
    pub heads: Vec<HeadParams>,
    step: u64,
}

/// Adam settings.
#[derive(Debug, Clone, Copy)]
pub struct Adam {
    pub lr: f32,
    pub beta1: f32,
    pub beta2: f32,
    pub eps: f32,
    /// Gradients are clipped to this global L2 norm before each step (0 = off).
    pub clip: f32,
}

impl Default for Adam {
    fn default() -> Self {
        Adam {
            lr: 1e-3,
            beta1: 0.9,
            beta2: 0.999,
            eps: 1e-8,
            clip: 1.0,
        }
    }
}

/// Seeded xorshift64* with approximately normal samples.
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0 | 1;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    pub fn unit(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32 + 0.5) / (1u64 << 24) as f32
    }
    /// N(0, 1) approximated by the sum of 12 uniforms minus 6 (Irwin–Hall: mean 0, variance 1).
    /// Only exact operations: Box–Muller's `ln` and `cos` come from the platform math library,
    /// whose last bit differs between operating systems, so the same seed gave different initial
    /// weights (and different trained models) on Linux, Windows and macOS.
    pub fn normal(&mut self) -> f32 {
        (0..12).map(|_| self.unit()).sum::<f32>() - 6.0
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
}

impl TrainModel {
    /// Fresh weights: N(0, 0.02) matrices and embeddings, zero biases, unit LayerNorm gains.
    pub fn init(tier: &str, config: Config, heads: &[&str], seed: u64) -> Self {
        let mut rng = Rng(seed);
        let (d, f) = (config.d as usize, config.ffn as usize);
        let mut normal = |n: usize| Param::new((0..n).map(|_| 0.02 * rng.normal()).collect());
        let emb = normal(config.vocab as usize * d);
        let pos = normal(config.ctx as usize * d);
        let mut layers = Vec::new();
        for _ in 0..config.layers {
            layers.push(LayerParams {
                ln1_g: Param::new(vec![1.0; d]),
                ln1_b: Param::new(vec![0.0; d]),
                qkv_w: normal(3 * d * d),
                qkv_b: Param::new(vec![0.0; 3 * d]),
                out_w: normal(d * d),
                out_b: Param::new(vec![0.0; d]),
                ln2_g: Param::new(vec![1.0; d]),
                ln2_b: Param::new(vec![0.0; d]),
                ff1_w: normal(f * d),
                ff1_b: Param::new(vec![0.0; f]),
                ff2_w: normal(d * f),
                ff2_b: Param::new(vec![0.0; d]),
            });
        }
        let heads = heads
            .iter()
            .map(|n| HeadParams {
                name: n.to_string(),
                w: normal(d),
                b: Param::new(vec![0.0]),
            })
            .collect();
        TrainModel {
            tier: tier.to_string(),
            config,
            emb,
            pos,
            layers,
            lnf_g: Param::new(vec![1.0; d]),
            lnf_b: Param::new(vec![0.0; d]),
            heads,
            step: 0,
        }
    }

    pub fn head_index(&self, name: &str) -> Option<usize> {
        self.heads.iter().position(|h| h.name == name)
    }

    /// Every parameter, in a fixed order (Adam, gradient checks, export).
    pub fn params_mut(&mut self) -> Vec<&mut Param> {
        let mut v: Vec<&mut Param> = vec![&mut self.emb, &mut self.pos];
        for l in &mut self.layers {
            v.extend([
                &mut l.ln1_g,
                &mut l.ln1_b,
                &mut l.qkv_w,
                &mut l.qkv_b,
                &mut l.out_w,
                &mut l.out_b,
                &mut l.ln2_g,
                &mut l.ln2_b,
                &mut l.ff1_w,
                &mut l.ff1_b,
                &mut l.ff2_w,
                &mut l.ff2_b,
            ]);
        }
        v.extend([&mut self.lnf_g, &mut self.lnf_b]);
        for h in &mut self.heads {
            v.extend([&mut h.w, &mut h.b]);
        }
        v
    }

    pub fn zero_grad(&mut self) {
        for p in self.params_mut() {
            p.g.iter_mut().for_each(|g| *g = 0.0);
        }
    }

    /// One Adam step on the accumulated gradients (divided by `batch`), then zero them.
    pub fn adam_step(&mut self, opt: &Adam, batch: usize) {
        self.step += 1;
        let t = self.step as i32;
        let scale = 1.0 / batch.max(1) as f32;
        let mut norm2 = 0.0f64;
        for p in self.params_mut() {
            norm2 +=
                p.g.iter()
                    .map(|g| f64::from(g * scale).powi(2))
                    .sum::<f64>();
        }
        let norm = norm2.sqrt() as f32;
        let clip = if opt.clip > 0.0 && norm > opt.clip {
            opt.clip / norm
        } else {
            1.0
        };
        let (bc1, bc2) = (1.0 - opt.beta1.powi(t), 1.0 - opt.beta2.powi(t));
        for p in self.params_mut() {
            for i in 0..p.w.len() {
                let g = p.g[i] * scale * clip;
                p.m[i] = opt.beta1 * p.m[i] + (1.0 - opt.beta1) * g;
                p.v[i] = opt.beta2 * p.v[i] + (1.0 - opt.beta2) * g * g;
                p.w[i] -= opt.lr * (p.m[i] / bc1) / ((p.v[i] / bc2).sqrt() + opt.eps);
                p.g[i] = 0.0;
            }
        }
    }

    /// Every parameter, in the same order as [`TrainModel::params_mut`] and [`Grads`].
    pub fn params(&self) -> Vec<&Param> {
        let mut v: Vec<&Param> = vec![&self.emb, &self.pos];
        for l in &self.layers {
            v.extend([
                &l.ln1_g, &l.ln1_b, &l.qkv_w, &l.qkv_b, &l.out_w, &l.out_b, &l.ln2_g, &l.ln2_b,
                &l.ff1_w, &l.ff1_b, &l.ff2_w, &l.ff2_b,
            ]);
        }
        v.extend([&self.lnf_g, &self.lnf_b]);
        for h in &self.heads {
            v.extend([&h.w, &h.b]);
        }
        v
    }

    /// Pooled vector for token ids, with everything the backward pass needs.
    pub fn forward(&self, ids: &[u32]) -> (Vec<f32>, Cache) {
        let d = self.config.d as usize;
        let ids = &ids[..ids.len().min(self.config.ctx as usize)];
        let mut h: Vec<Vec<f32>> = ids
            .iter()
            .enumerate()
            .map(|(p, id)| {
                let t = (*id).min(self.config.vocab - 1) as usize;
                let e = &self.emb.w[t * d..(t + 1) * d];
                let q = &self.pos.w[p * d..(p + 1) * d];
                e.iter().zip(q).map(|(a, b)| a + b).collect()
            })
            .collect();
        let mut layers = Vec::with_capacity(self.layers.len());
        for l in &self.layers {
            let (out, c) = self.layer_forward(l, h);
            layers.push(c);
            h = out;
        }
        let (pooled, xhat, rstd) = ln_forward(&h[0], &self.lnf_g.w, &self.lnf_b.w);
        (
            pooled,
            Cache {
                ids: ids.to_vec(),
                layers,
                n: h.len(),
                f_xhat: xhat,
                f_rstd: rstd,
            },
        )
    }

    fn layer_forward(&self, l: &LayerParams, x: Vec<Vec<f32>>) -> (Vec<Vec<f32>>, LayerCache) {
        let c = self.config;
        let (d, f, nh) = (c.d as usize, c.ffn as usize, c.heads as usize);
        let hd = d / nh;
        let inv = 1.0 / (hd as f32).sqrt();
        let n = x.len();
        let mut ln1 = Vec::with_capacity(n);
        let mut a = Vec::with_capacity(n);
        for xi in &x {
            let (y, xh, r) = ln_forward(xi, &l.ln1_g.w, &l.ln1_b.w);
            a.push(y);
            ln1.push((xh, r));
        }
        let qkv = linear_rows(&l.qkv_w.w, &l.qkv_b.w, &a, 3 * d, d);
        let mut probs = vec![vec![vec![0.0f32; n]; n]; nh];
        let mut ctx = vec![vec![0.0f32; d]; n];
        for (hh, ph) in probs.iter_mut().enumerate() {
            let (qs, ks, vs) = (hh * hd, d + hh * hd, 2 * d + hh * hd);
            for i in 0..n {
                let q = &qkv[i][qs..qs + hd];
                for j in 0..n {
                    ph[i][j] = dot_f32(q, &qkv[j][ks..ks + hd]) * inv;
                }
                softmax(&mut ph[i]);
                for j in 0..n {
                    axpy(&mut ctx[i][qs..qs + hd], ph[i][j], &qkv[j][vs..vs + hd]);
                }
            }
        }
        let o = linear_rows(&l.out_w.w, &l.out_b.w, &ctx, d, d);
        let x1: Vec<Vec<f32>> = x
            .iter()
            .zip(&o)
            .map(|(u, v)| u.iter().zip(v).map(|(p, q)| p + q).collect())
            .collect();
        let mut ln2 = Vec::with_capacity(n);
        let mut b2 = Vec::with_capacity(n);
        for xi in &x1 {
            let (y, xh, r) = ln_forward(xi, &l.ln2_g.w, &l.ln2_b.w);
            b2.push(y);
            ln2.push((xh, r));
        }
        let pre = linear_rows(&l.ff1_w.w, &l.ff1_b.w, &b2, f, d);
        let post: Vec<Vec<f32>> = pre
            .iter()
            .map(|p| p.iter().map(|v| gelu(*v)).collect())
            .collect();
        let o2 = linear_rows(&l.ff2_w.w, &l.ff2_b.w, &post, d, f);
        let out = x1
            .iter()
            .zip(&o2)
            .map(|(u, v)| u.iter().zip(v).map(|(p, q)| p + q).collect())
            .collect();
        (
            out,
            LayerCache {
                a,
                ln1,
                qkv,
                probs,
                ctx,
                ln2,
                b2,
                pre,
                post,
            },
        )
    }

    /// Back-propagates `dpooled` (gradient w.r.t. the pooled vector) into `g`.
    pub fn backward_into(&self, cache: &Cache, dpooled: &[f32], g: &mut Grads) {
        let d = self.config.d as usize;
        let n = cache.n;
        let nl = self.layers.len();
        let mut dh = vec![vec![0.0f32; d]; n];
        {
            let (dg, db) = g.pair(slot_lnf(nl));
            dh[0] = ln_backward(dpooled, &cache.f_xhat, cache.f_rstd, &self.lnf_g.w, dg, db);
        }
        for li in (0..nl).rev() {
            dh = self.layer_backward(li, &cache.layers[li], dh, g);
        }
        let (de, dp) = g.pair(0);
        for (p, (id, gr)) in cache.ids.iter().zip(&dh).enumerate() {
            let t = (*id).min(self.config.vocab - 1) as usize;
            axpy(&mut de[t * d..(t + 1) * d], 1.0, gr);
            axpy(&mut dp[p * d..(p + 1) * d], 1.0, gr);
        }
    }

    fn layer_backward(
        &self,
        li: usize,
        c: &LayerCache,
        dout: Vec<Vec<f32>>,
        g: &mut Grads,
    ) -> Vec<Vec<f32>> {
        let cfg = self.config;
        let (d, f, nh) = (cfg.d as usize, cfg.ffn as usize, cfg.heads as usize);
        let hd = d / nh;
        let inv = 1.0 / (hd as f32).sqrt();
        let n = dout.len();
        let l = &self.layers[li];
        let s = slot_layer(li);
        // Feed-forward block: out = x1 + W2·gelu(W1·LN2(x1)).
        let dgl = {
            let (dw, db) = g.pair(s + 10);
            linear_backward_rows(&l.ff2_w.w, &c.post, &dout, dw, db, d, f)
        };
        let dp: Vec<Vec<f32>> = dgl
            .iter()
            .zip(&c.pre)
            .map(|(gr, x)| gr.iter().zip(x).map(|(a, b)| a * gelu_grad(*b)).collect())
            .collect();
        let db2 = {
            let (dw, db) = g.pair(s + 8);
            linear_backward_rows(&l.ff1_w.w, &c.b2, &dp, dw, db, f, d)
        };
        let mut dx1 = dout;
        {
            let (dg, db) = g.pair(s + 6);
            for i in 0..n {
                let dln = ln_backward(&db2[i], &c.ln2[i].0, c.ln2[i].1, &l.ln2_g.w, dg, db);
                axpy(&mut dx1[i], 1.0, &dln);
            }
        }
        // Attention block: x1 = x + Wo·attn(LN1(x)).
        let dctx = {
            let (dw, db) = g.pair(s + 4);
            linear_backward_rows(&l.out_w.w, &c.ctx, &dx1, dw, db, d, d)
        };
        let mut dqkv = vec![vec![0.0f32; 3 * d]; n];
        for h in 0..nh {
            let p = &c.probs[h];
            let (qs, ks, vs) = (h * hd, d + h * hd, 2 * d + h * hd);
            let mut dq = vec![vec![0.0f32; hd]; n];
            let mut dk = vec![vec![0.0f32; hd]; n];
            let mut dv = vec![vec![0.0f32; hd]; n];
            let mut dpr = vec![0.0f32; n];
            for i in 0..n {
                let dci = &dctx[i][qs..qs + hd];
                // dp_ij = dctx_i · v_j ;  dv_j += p_ij dctx_i
                for j in 0..n {
                    dpr[j] = dot_f32(dci, &c.qkv[j][vs..vs + hd]);
                    axpy(&mut dv[j], p[i][j], dci);
                }
                // softmax backward: ds_ij = p_ij (dp_ij - Σ_k p_ik dp_ik)
                let sum: f32 = (0..n).map(|j| p[i][j] * dpr[j]).sum();
                for j in 0..n {
                    let ds = p[i][j] * (dpr[j] - sum) * inv;
                    if ds == 0.0 {
                        continue;
                    }
                    axpy(&mut dq[i], ds, &c.qkv[j][ks..ks + hd]);
                    axpy(&mut dk[j], ds, &c.qkv[i][qs..qs + hd]);
                }
            }
            for i in 0..n {
                dqkv[i][qs..qs + hd].copy_from_slice(&dq[i]);
                dqkv[i][ks..ks + hd].copy_from_slice(&dk[i]);
                dqkv[i][vs..vs + hd].copy_from_slice(&dv[i]);
            }
        }
        let da = {
            let (dw, db) = g.pair(s + 2);
            linear_backward_rows(&l.qkv_w.w, &c.a, &dqkv, dw, db, 3 * d, d)
        };
        let mut dx = dx1;
        let (dg, db) = g.pair(s);
        for i in 0..n {
            let dln = ln_backward(&da[i], &c.ln1[i].0, c.ln1[i].1, &l.ln1_g.w, dg, db);
            axpy(&mut dx[i], 1.0, &dln);
        }
        dx
    }

    /// Score of a pooled vector under head `head`.
    pub fn head_score(&self, head: usize, pooled: &[f32]) -> f32 {
        dot(&self.heads[head].w.w, pooled) + self.heads[head].b.w[0]
    }

    /// Listwise ranking loss for one finding: candidates' scores and a fixed "none" score of
    /// 0 go through a softmax; the target is `answer` (None = "none of these"). `base`, if
    /// not empty, is added to the candidates' scores first (a tier trained as a residual on
    /// another ranker's scores). Adds the gradients to `g` and returns the loss.
    pub fn rank_loss_into(
        &self,
        head: usize,
        inputs: &[Vec<u32>],
        base: &[f32],
        answer: Option<usize>,
        g: &mut Grads,
    ) -> f32 {
        let fwd: Vec<(Vec<f32>, Cache)> = inputs.iter().map(|ids| self.forward(ids)).collect();
        let mut scores: Vec<f32> = fwd
            .iter()
            .enumerate()
            .map(|(j, (p, _))| self.head_score(head, p) + base.get(j).copied().unwrap_or(0.0))
            .collect();
        scores.push(0.0); // "none of these"
        let mut probs = scores.clone();
        softmax(&mut probs);
        let target = answer.unwrap_or(inputs.len());
        let loss = -probs[target].max(1e-12).ln();
        let hs = slot_head(self.layers.len(), head);
        for (j, (pooled, cache)) in fwd.iter().enumerate() {
            let ds = probs[j] - if j == target { 1.0 } else { 0.0 };
            if ds == 0.0 {
                continue;
            }
            {
                let (gw, gb) = g.pair(hs);
                axpy(gw, ds, pooled);
                gb[0] += ds;
            }
            let dpooled: Vec<f32> = self.heads[head].w.w.iter().map(|w| ds * w).collect();
            self.backward_into(cache, &dpooled, g);
        }
        loss
    }

    /// [`TrainModel::rank_loss_into`] accumulating into the parameters' own gradients.
    pub fn rank_loss(&mut self, head: usize, inputs: &[Vec<u32>], answer: Option<usize>) -> f32 {
        let mut g = Grads::zeros_like(self);
        let loss = self.rank_loss_into(head, inputs, &[], answer, &mut g);
        g.add_to(self);
        loss
    }

    /// One optimisation step on a batch of findings, computed on [`SHARDS`] threads. The batch
    /// is cut into at most `SHARDS` contiguous parts; each part's gradient is summed in order
    /// and the parts are added in order, so the result does not depend on the machine's
    /// number of cores. Returns the summed loss.
    pub fn train_step(
        &mut self,
        head: usize,
        batch: &[Example],
        opt: &Adam,
    ) -> f32 {
        if batch.is_empty() {
            return 0.0;
        }
        let part = batch.len().div_ceil(SHARDS);
        let model = &*self;
        let parts: Vec<(Grads, f32)> = std::thread::scope(|s| {
            let hs: Vec<_> = batch
                .chunks(part)
                .map(|chunk| {
                    s.spawn(move || {
                        let mut g = Grads::zeros_like(model);
                        let mut loss = 0.0f32;
                        for e in chunk {
                            loss += model.rank_loss_into(head, &e.inputs, &e.base, e.answer, &mut g);
                        }
                        (g, loss)
                    })
                })
                .collect();
            hs.into_iter()
                .map(|h| h.join().expect("training thread panicked"))
                .collect()
        });
        let mut loss = 0.0;
        for (g, l) in &parts {
            g.add_to(self);
            loss += l;
        }
        self.adam_step(opt, batch.len());
        loss
    }
}

/// One training finding: the candidates' token ids, optional base scores (see
/// [`TrainModel::rank_loss_into`]) and the answer (None = "none of these").
#[derive(Debug, Clone)]
pub struct Example {
    pub inputs: Vec<Vec<u32>>,
    pub base: Vec<f32>,
    pub answer: Option<usize>,
}

/// Number of gradient shards (and threads) per training step. Fixed, so that results do not
/// depend on the number of cores.
pub const SHARDS: usize = 8;

/// Gradient buffers, one per parameter, in [`TrainModel::params`] order.
#[derive(Debug, Clone)]
pub struct Grads {
    pub slots: Vec<Vec<f32>>,
}

impl Grads {
    pub fn zeros_like(m: &TrainModel) -> Self {
        Grads {
            slots: m.params().iter().map(|p| vec![0.0; p.w.len()]).collect(),
        }
    }

    /// Slots `i` and `i + 1` (a weight and its bias, or a gain and its bias).
    fn pair(&mut self, i: usize) -> (&mut [f32], &mut [f32]) {
        let (a, b) = self.slots.split_at_mut(i + 1);
        (&mut a[i], &mut b[0])
    }

    /// Adds these gradients to the model's own (`Param::g`).
    pub fn add_to(&self, m: &mut TrainModel) {
        for (p, g) in m.params_mut().into_iter().zip(&self.slots) {
            axpy(&mut p.g, 1.0, g);
        }
    }
}

fn slot_layer(li: usize) -> usize {
    2 + 12 * li
}
fn slot_lnf(layers: usize) -> usize {
    2 + 12 * layers
}
fn slot_head(layers: usize, h: usize) -> usize {
    4 + 12 * layers + 2 * h
}


impl TrainModel {
    /// The int8 runtime model: matrices and embeddings quantised per row (symmetric, scale =
    /// max|row| / 127), LayerNorm parameters, biases and heads kept in f32.
    pub fn export(&self, tokenizer: crate::Tokenizer, meta: &str) -> crate::Model {
        use crate::net::{Head, Layer, Model, Norm, Q8};
        let c = self.config;
        let (d, f) = (c.d as usize, c.ffn as usize);
        let q8 = |p: &Param, rows: usize, cols: usize| {
            let mut data = Vec::with_capacity(rows * cols);
            let mut scale = Vec::with_capacity(rows);
            for r in 0..rows {
                let row = &p.w[r * cols..(r + 1) * cols];
                let amax = row.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                let s = if amax > 0.0 { amax / 127.0 } else { 1.0 };
                data.extend(
                    row.iter()
                        .map(|v| (v / s).round().clamp(-127.0, 127.0) as i8),
                );
                scale.push(s);
            }
            Q8 {
                rows,
                cols,
                data,
                scale,
            }
        };
        let norm = |g: &Param, b: &Param| Norm {
            gain: g.w.clone(),
            bias: b.w.clone(),
        };
        Model {
            tier: self.tier.clone(),
            config: c,
            tokenizer,
            emb: q8(&self.emb, c.vocab as usize, d),
            pos: q8(&self.pos, c.ctx as usize, d),
            layers: self
                .layers
                .iter()
                .map(|l| Layer {
                    ln1: norm(&l.ln1_g, &l.ln1_b),
                    qkv: q8(&l.qkv_w, 3 * d, d),
                    qkv_b: l.qkv_b.w.clone(),
                    out: q8(&l.out_w, d, d),
                    out_b: l.out_b.w.clone(),
                    ln2: norm(&l.ln2_g, &l.ln2_b),
                    ff1: q8(&l.ff1_w, f, d),
                    ff1_b: l.ff1_b.w.clone(),
                    ff2: q8(&l.ff2_w, d, f),
                    ff2_b: l.ff2_b.w.clone(),
                })
                .collect(),
            ln_f: norm(&self.lnf_g, &self.lnf_b),
            heads: self
                .heads
                .iter()
                .map(|h| Head {
                    name: h.name.clone(),
                    w: h.w.w.clone(),
                    b: h.b.w[0],
                })
                .collect(),
            meta: meta.to_string(),
        }
    }

    /// f32 score of `text` under head `head` (the reference the int8 export is compared to).
    pub fn score_text(&self, tokenizer: &crate::Tokenizer, head: usize, text: &str) -> f32 {
        let ids = tokenizer.encode(text, self.config.ctx as usize);
        let (pooled, _) = self.forward(&ids);
        self.head_score(head, &pooled)
    }
}

/// Activations saved by [`TrainModel::forward`] for the backward pass.
pub struct Cache {
    ids: Vec<u32>,
    layers: Vec<LayerCache>,
    n: usize,
    f_xhat: Vec<f32>,
    f_rstd: f32,
}

struct LayerCache {
    a: Vec<Vec<f32>>,
    ln1: Vec<(Vec<f32>, f32)>,
    qkv: Vec<Vec<f32>>,
    probs: Vec<Vec<Vec<f32>>>,
    ctx: Vec<Vec<f32>>,
    ln2: Vec<(Vec<f32>, f32)>,
    b2: Vec<Vec<f32>>,
    pre: Vec<Vec<f32>>,
    post: Vec<Vec<f32>>,
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    dot_f32(a, b)
}

/// y += a·x, element by element (vectorises; no reordering of sums).
fn axpy(y: &mut [f32], a: f32, x: &[f32]) {
    for (yi, xi) in y.iter_mut().zip(x) {
        *yi += a * xi;
    }
}

/// Y = X Wᵀ + b for every row of X, W row-major (rows × cols). Loops over weight rows
/// outermost, so each row stays in cache while all tokens use it.
fn linear_rows(w: &[f32], b: &[f32], xs: &[Vec<f32>], rows: usize, cols: usize) -> Vec<Vec<f32>> {
    let mut y = vec![vec![0.0f32; rows]; xs.len()];
    for r in 0..rows {
        let wr = &w[r * cols..(r + 1) * cols];
        for (yi, x) in y.iter_mut().zip(xs) {
            yi[r] = dot_f32(wr, x) + b[r];
        }
    }
    y
}

/// For every row: dW += dy xᵀ and db += dy; returns dX = dY W.
fn linear_backward_rows(
    w: &[f32],
    xs: &[Vec<f32>],
    dys: &[Vec<f32>],
    dw: &mut [f32],
    db: &mut [f32],
    rows: usize,
    cols: usize,
) -> Vec<Vec<f32>> {
    let mut dx = vec![vec![0.0f32; cols]; xs.len()];
    for r in 0..rows {
        let wr = &w[r * cols..(r + 1) * cols];
        let dwr = &mut dw[r * cols..(r + 1) * cols];
        for (i, x) in xs.iter().enumerate() {
            let g = dys[i][r];
            if g == 0.0 {
                continue;
            }
            db[r] += g;
            axpy(dwr, g, x);
            axpy(&mut dx[i], g, wr);
        }
    }
    dx
}

/// LayerNorm (eps 1e-5, as the runtime) → (y, x̂, 1/σ).
fn ln_forward(x: &[f32], gain: &[f32], bias: &[f32]) -> (Vec<f32>, Vec<f32>, f32) {
    let n = x.len() as f32;
    let mean = x.iter().sum::<f32>() / n;
    let var = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n;
    let rstd = 1.0 / (var + 1e-5).sqrt();
    let xhat: Vec<f32> = x.iter().map(|v| (v - mean) * rstd).collect();
    let y = xhat
        .iter()
        .zip(gain.iter().zip(bias))
        .map(|(h, (g, b))| h * g + b)
        .collect();
    (y, xhat, rstd)
}

fn ln_backward(
    dy: &[f32],
    xhat: &[f32],
    rstd: f32,
    gain: &[f32],
    dgain: &mut [f32],
    dbias: &mut [f32],
) -> Vec<f32> {
    let n = dy.len() as f32;
    let mut dxhat = vec![0.0f32; dy.len()];
    for i in 0..dy.len() {
        dgain[i] += dy[i] * xhat[i];
        dbias[i] += dy[i];
        dxhat[i] = dy[i] * gain[i];
    }
    let m1 = dxhat.iter().sum::<f32>() / n;
    let m2 = dxhat.iter().zip(xhat).map(|(a, b)| a * b).sum::<f32>() / n;
    dxhat
        .iter()
        .zip(xhat)
        .map(|(g, h)| rstd * (g - m1 - h * m2))
        .collect()
}

/// d/dx of the tanh-approximated GELU used by the runtime.
fn gelu_grad(x: f32) -> f32 {
    const C: f32 = 0.797_884_6;
    let u = C * (x + 0.044_715 * x * x * x);
    let t = tanh(u);
    0.5 * (1.0 + t) + 0.5 * x * (1.0 - t * t) * C * (1.0 + 3.0 * 0.044_715 * x * x)
}

fn softmax(x: &mut [f32]) {
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


#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> TrainModel {
        let c = Config {
            vocab: 270,
            d: 8,
            layers: 2,
            heads: 2,
            ffn: 12,
            ctx: 12,
        };
        let mut m = TrainModel::init("t", c, &["rank"], 11);
        // Non-trivial LayerNorm gains/biases so their gradients are exercised too.
        let mut r = Rng(5);
        for p in m.params_mut() {
            for w in &mut p.w {
                *w += 0.05 * r.normal();
            }
        }
        m
    }

    fn loss_of(m: &mut TrainModel, inputs: &[Vec<u32>], answer: Option<usize>) -> f32 {
        let l = m.rank_loss(0, inputs, answer);
        m.zero_grad();
        l
    }

    /// Analytic gradients agree with central finite differences for every parameter group.
    #[test]
    fn gradients_match_finite_differences() {
        let inputs = vec![
            vec![256, 97, 98, 99, 3],
            vec![256, 7, 8],
            vec![256, 97, 98, 120, 121, 4],
        ];
        for answer in [Some(1), None] {
            let mut m = tiny();
            m.zero_grad();
            m.rank_loss(0, &inputs, answer);
            let analytic: Vec<Vec<f32>> = m.params_mut().iter().map(|p| p.g.clone()).collect();
            m.zero_grad();
            let mut checked = 0;
            for (gi, grad) in analytic.iter().enumerate() {
                // up to 6 entries per group with a non-negligible gradient
                let picks: Vec<usize> = (0..grad.len())
                    .filter(|&i| grad[i].abs() > 1e-4)
                    .take(6)
                    .collect();
                for i in picks {
                    let eps = 1e-2f32;
                    let orig = m.params_mut()[gi].w[i];
                    m.params_mut()[gi].w[i] = orig + eps;
                    let lp = loss_of(&mut m, &inputs, answer);
                    m.params_mut()[gi].w[i] = orig - eps;
                    let lm = loss_of(&mut m, &inputs, answer);
                    m.params_mut()[gi].w[i] = orig;
                    let num = (lp - lm) / (2.0 * eps);
                    let a = grad[i];
                    let err = (num - a).abs() / (num.abs().max(a.abs()).max(1e-3));
                    assert!(
                        err < 0.08,
                        "group {gi} index {i}: analytic {a} vs numeric {num} (answer {answer:?})"
                    );
                    checked += 1;
                }
            }
            assert!(checked > 60, "only {checked} gradient entries checked");
        }
    }

    /// The model learns a toy ranking task: pick the candidate equal to the query, or "none".
    #[test]
    fn learns_a_toy_ranking_task() {
        let c = Config {
            vocab: 270,
            d: 16,
            layers: 1,
            heads: 2,
            ffn: 32,
            ctx: 16,
        };
        let mut m = TrainModel::init("t", c, &["rank"], 3);
        let opt = Adam {
            lr: 3e-3,
            ..Adam::default()
        };
        let mut rng = Rng(9);
        let mut make = |rng: &mut Rng| {
            let q = 97 + rng.below(8) as u32;
            let offered = rng.below(3) != 0;
            let mut cands: Vec<u32> = (97..105).filter(|x| *x != q).collect();
            cands.truncate(3);
            let answer = if offered {
                let pos = rng.below(4);
                cands.insert(pos, q);
                Some(pos)
            } else {
                None
            };
            let inputs: Vec<Vec<u32>> = cands.iter().map(|cand| vec![256, q, 32, *cand]).collect();
            (inputs, answer)
        };
        type Case = (Vec<Vec<u32>>, Option<usize>);
        let eval = |m: &mut TrainModel, rng: &mut Rng, make: &mut dyn FnMut(&mut Rng) -> Case| {
            let mut s = 0.0;
            for _ in 0..60 {
                let (x, a) = make(rng);
                s += loss_of(m, &x, a);
            }
            s / 60.0
        };
        let before = eval(&mut m, &mut Rng(100), &mut make);
        for _ in 0..300 {
            for _ in 0..8 {
                let (x, a) = make(&mut rng);
                m.rank_loss(0, &x, a);
            }
            m.adam_step(&opt, 8);
        }
        let after = eval(&mut m, &mut Rng(100), &mut make);
        assert!(after < 0.5 * before, "loss {before} -> {after}");
    }

    /// Toy task: the query is one letter; the right candidate is the same letter, or "none"
    /// when it is not offered. Inputs are "q c" as bytes, so a merge-free tokenizer reproduces
    /// exactly the ids the model was trained on.
    fn toy_case(rng: &mut Rng) -> (u8, Vec<u8>, Option<usize>) {
        let q = b'a' + rng.below(8) as u8;
        let mut cands: Vec<u8> = (b'a'..b'i').filter(|x| *x != q).collect();
        cands.truncate(3);
        let answer = (rng.below(3) != 0).then(|| {
            let pos = rng.below(4);
            cands.insert(pos, q);
            pos
        });
        (q, cands, answer)
    }

    fn toy_ids(q: u8, c: u8) -> Vec<u32> {
        vec![crate::tokenizer::CLS, u32::from(q), 32, u32::from(c)]
    }

    fn pick(scores: &[f32]) -> Option<usize> {
        // argmax over the candidates and the fixed "none" score 0 (None = abstain)
        scores
            .iter()
            .enumerate()
            .fold(
                (None, 0.0f32),
                |acc, (i, s)| if *s > acc.1 { (Some(i), *s) } else { acc },
            )
            .0
    }

    /// A trained model exported to int8 loads in the runtime and makes the same decision
    /// (which candidate, or none) as the f32 model it came from.
    #[test]
    fn int8_export_keeps_the_trained_decisions() {
        let c = Config {
            vocab: 270,
            d: 16,
            layers: 1,
            heads: 2,
            ffn: 32,
            ctx: 16,
        };
        let mut m = TrainModel::init("t", c, &["rank"], 3);
        let opt = Adam {
            lr: 3e-3,
            ..Adam::default()
        };
        let mut rng = Rng(9);
        for _ in 0..400 {
            for _ in 0..8 {
                let (q, cands, a) = toy_case(&mut rng);
                let x: Vec<Vec<u32>> = cands.iter().map(|c| toy_ids(q, *c)).collect();
                m.rank_loss(0, &x, a);
            }
            m.adam_step(&opt, 8);
        }
        let tok = crate::Tokenizer::new(Vec::new());
        let bytes = crate::format::to_bytes(&m.export(tok.clone(), "{}"));
        let (rt, _) = crate::format::from_bytes(&bytes).unwrap();
        let mut eval = Rng(100);
        let (mut same, mut right_f32, mut right_int8, n) = (0, 0, 0, 300);
        for _ in 0..n {
            let (q, cands, a) = toy_case(&mut eval);
            let f: Vec<f32> = cands
                .iter()
                .map(|c| m.head_score(0, &m.forward(&toy_ids(q, *c)).0))
                .collect();
            let i: Vec<f32> = cands
                .iter()
                .map(|c| {
                    rt.score("rank", std::str::from_utf8(&[q, b' ', *c]).unwrap())
                        .unwrap()
                })
                .collect();
            same += usize::from(pick(&f) == pick(&i));
            right_f32 += usize::from(pick(&f) == a);
            right_int8 += usize::from(pick(&i) == a);
        }
        // Always abstaining scores ~1/3. This 1-layer, d=16 model learns letter priors rather
        // than full query/candidate comparison (~75%); that is enough to test the export.
        eprintln!("int8 export: same decision {same}/{n}; correct f32 {right_f32}/{n}, int8 {right_int8}/{n}");
        assert!(
            right_f32 as f32 > 0.6 * n as f32,
            "toy model did not learn: {right_f32}/{n}"
        );
        assert!(
            same as f32 >= 0.97 * n as f32,
            "int8 changed {} of {n} decisions",
            n - same
        );
        assert!(
            right_int8 + 3 >= right_f32,
            "int8 accuracy {right_int8} vs f32 {right_f32}"
        );
    }

    /// The threaded step gives the same bits as computing the same shards one after another.
    #[test]
    fn threaded_step_equals_serial_shards() {
        let batch: Vec<Example> = (0..13)
            .map(|i| Example {
                inputs: vec![vec![256, 97 + i, 98], vec![256, 99, i], vec![256, 100 + i % 3]],
                base: vec![0.1 * i as f32, -0.2, 0.0],
                answer: if i % 3 == 0 { None } else { Some((i % 3) as usize) },
            })
            .collect();
        let opt = Adam::default();
        let mut a = tiny();
        let mut b = tiny();
        for _ in 0..2 {
            a.train_step(0, &batch, &opt);
            for chunk in batch.chunks(batch.len().div_ceil(SHARDS)) {
                let mut g = Grads::zeros_like(&b);
                for e in chunk {
                    b.rank_loss_into(0, &e.inputs, &e.base, e.answer, &mut g);
                }
                g.add_to(&mut b);
            }
            b.adam_step(&opt, batch.len());
        }
        let bits = |m: &mut TrainModel| -> Vec<u32> {
            m.params_mut()
                .iter()
                .flat_map(|p| p.w.iter().map(|v| v.to_bits()))
                .collect()
        };
        assert_eq!(bits(&mut a), bits(&mut b));
    }

    #[test]
    fn training_is_reproducible() {
        let run = || {
            let mut m = tiny();
            let x = vec![vec![256, 97, 98], vec![256, 99]];
            for _ in 0..3 {
                m.rank_loss(0, &x, Some(0));
                m.adam_step(&Adam::default(), 1);
            }
            m.emb.w.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }
}
