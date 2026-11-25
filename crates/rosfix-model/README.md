# rosfix-model

Runtime and trainer for the optional, downloadable assist models of
[rosfix](https://github.com/SohamWalam11/ros2-safety-lint), a static checker for ROS 2
misconfigurations.

When rosfix finds a mistake such as a misspelled parameter name, it builds a closed set of
valid replacements. `rosfix fix --assist` then picks one, or abstains, and verifies the edit
by re-analysing the workspace. The default picker is a 321-number model compiled into
rosfix. This crate runs larger pickers ("tiers"), which also read the rule that fired and the
source lines around the mistake. A tier only **scores** the candidates rosfix offers against a
fixed "none of these" score. It never generates text.

> **Status:** no tier has been released yet. A tier is released only if it beats the built-in
> ranker, with no more wrong edits, in every fold of rosfix's name-disjoint cross-validation.
> Until then `rosfix --model nano|mini|base` falls back to the built-in ranker, and tiers can
> only be tried from a file path.

## Properties

- **No dependencies, CPU only.** No GPU, no network, no Python, no ML framework.
- **int8 weights**, one scale per row. Weights and activations meet in int8 × int8 → i32 dot
  products, which are exact. An AVX2 path (selected at run time) and a portable scalar path
  give identical bits.
- **Bit-identical results on every machine.** Floating-point code runs in a fixed order,
  without fused multiply-add. `exp` and `tanh` are built from IEEE-exact operations instead
  of the platform math library, whose last bit differs between operating systems.
- **Checked model files.** Every `.rfm` file ends with a SHA-256 of its contents. Every
  tensor shape is validated on load, and files over 256 MiB are refused.

## Tiers

Every tier is the same pre-LayerNorm transformer encoder over an 8,192-token byte-level BPE
vocabulary, with [CLS] pooling and linear scoring heads.

| tier | layers / width / heads | context | stored numbers | file | per candidate, 64 tokens* |
|---|---|---:|---:|---:|---:|
| nano | 2 / 128 / 2 | 128 | 1.46 M | 1.6 MB | 5 ms |
| mini | 4 / 256 / 4 | 256 | 5.32 M | 5.5 MB | 25 ms |
| base | 6 / 512 / 8 | 384 | 23.3 M | 23.6 MB | 85 ms |

\* Median of 5 trials, one thread, Intel Core 5 120U laptop under normal background load.
`rosfix-model-bench` measures your machine.

## Usage

```rust
use rosfix_model::format;
use std::path::Path;

fn main() -> Result<(), String> {
    // Returns the model and the SHA-256 of the file.
    let (model, sha256) = format::load(Path::new("nano.rfm"))?;
    println!("{} ({}), {} numbers", model.tier, &sha256[..8], model.params());

    // rosfix's input format: rule, the wrong text, one candidate, source context.
    let text = "[RULE] P003 [CUR] max_velocty [CAND] max_velocity [CTX] controller_server:";
    let score = model.score("rank", text).ok_or("model has no `rank` head")?;
    println!("score {score:.3}");
    Ok(())
}
```

Inside rosfix, `--model <tier|path>` does this for every candidate. It picks a candidate
only if its softmax probability (taken together with the fixed "none" score of 0) is at least
0.5 and at least 0.2 above the runner-up.

## Modules

- `net`: the int8 encoder (`Model`, `Config`, `Q8`) and its deterministic math.
- `format`: the `.rfm` file format (`load`, `save`, `to_bytes`, `from_bytes`).
- `tokenizer`: byte-level BPE (ids 0–255 are bytes, 256 is [CLS], merges start at 257).
  Training updates pair counts incrementally and is tested to equal the naive algorithm.
- `train`: the f32 twin of the encoder, with a hand-written backward pass (checked against
  finite differences), Adam, a listwise ranking loss with a fixed "none" score, and export to
  int8. A training step runs on a fixed number of threads, and their gradients are added in a
  fixed order. A seed therefore gives the same model on every machine, whatever its number of
  cores or SIMD level.
- `sha256`: FIPS 180-4 SHA-256, tested against the NIST vectors.

## Tools

- `rosfix-model-bench [--queries N] [--tier nano|mini|base]`: size, load time and latency
  per scored query for each tier shape on this machine.
- `rosfix-model-bpe [--merges N] [--out merges.txt] --repos-in DIR`: learns the tokenizer
  from the ROS 2 configuration files (YAML, launch, URDF/xacro/SRDF, `package.xml`) of a
  directory of repositories. It holds out one repository in ten and reports bytes per token
  on them.

The ranking data and the training and evaluation driver (`rosfix-train --dump-tier`,
`rosfix-tier`) live in the rosfix repository. Design, measurements and results are in the
paper (`paper/rosfix.tex`, section "Downloadable Assist Tiers").

## License

MIT OR Apache-2.0
