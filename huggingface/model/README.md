---
license: apache-2.0
language:
  - en
library_name: rosfix
pipeline_tag: text-ranking
tags:
  - ros2
  - robotics
  - static-analysis
  - rust
  - configuration
  - program-repair
datasets:
  - SohamW11/rosfix-knowledge-packs
model-index:
  - name: rosfix-assist
    results:
      - task:
          type: text-ranking
          name: Repair-value ranking with abstention
        dataset:
          type: SohamW11/rosfix-knowledge-packs
          name: rosfix synthetic corruptions, held-out packages and names, fold 0 (answer offered, n=1062)
          split: test
        metrics:
          - type: accuracy
            name: Correct picks (learned)
            value: 69.49
          - type: accuracy
            name: Correct picks (lexical baseline)
            value: 54.14
          - type: error_rate
            name: Wrong picks (learned)
            value: 0.19
      - task:
          type: text-ranking
          name: Abstention when the answer is absent
        dataset:
          type: SohamW11/rosfix-knowledge-packs
          name: rosfix synthetic corruptions, held-out packages and names, fold 0 (answer absent, n=729)
          split: test
        metrics:
          - type: accuracy
            name: Correct abstentions (learned)
            value: 99.31
          - type: accuracy
            name: Correct abstentions (lexical baseline)
            value: 96.57
      - task:
          type: text-ranking
          name: Wrong edits over all cases
        dataset:
          type: SohamW11/rosfix-knowledge-packs
          name: rosfix synthetic corruptions, held-out packages and names, fold 0 (all, n=1791)
          split: test
        metrics:
          - type: error_rate
            name: Wrong edits (learned)
            value: 0.39
          - type: error_rate
            name: Wrong edits (lexical baseline)
            value: 1.68
      - task:
          type: text-ranking
          name: 5-fold cross-validation (pooled)
        dataset:
          type: SohamW11/rosfix-knowledge-packs
          name: rosfix synthetic corruptions, 5 package folds, name-disjoint (n=3005)
          split: test
        metrics:
          - type: accuracy
            name: Correct (learned)
            value: 43.19
          - type: accuracy
            name: Correct (lexical baseline)
            value: 26.66
          - type: error_rate
            name: Wrong edits (learned)
            value: 1.60
          - type: error_rate
            name: Wrong edits (lexical baseline)
            value: 1.96
---

# rosfix-assist: fix ranker for ROS 2 configuration errors

The built-in fix assistant of [rosfix](https://github.com/SohamWalam11/ros2-safety-lint), a
static checker for silent ROS 2 misconfigurations. Licensed under Apache-2.0 or MIT, at your
option.

When rosfix finds a value with no mechanical fix (for example `max_lin_vel` for a node that
declares `max_linear_velocity`), the finding carries the closed set of valid names: declared
parameters, joints, frames, executables, controllers. This model ranks that set and either
picks one option or **abstains**. It never generates text. Every pick is verified by
re-analysing the workspace, and is written only with `rosfix fix --write --accept-assist`.

## Model

| | |
|---|---|
| Architecture | 16 hand-built string features → MLP 16→16→1 (tanh), softmax over the options plus a fixed "none of these" score |
| Parameters | 289 weights + 32 normalisation constants (≈1.3 KB as f32) |
| Runtime | Pure Rust, compiled into the rosfix binary; no ML runtime, no GPU, no network |
| Files | `learned_weights.rs` (the weights), `learned.rs` (features and inference) |

`learned.rs` is an **excerpt** of the rosfix source, included so you can read exactly how the
features are computed and the weights applied. It does not compile on its own: it uses
`super::lexical` (the ROS abbreviation table and lexical ranker) and
`crate::util::edit_distance`. The full, buildable module is in the rosfix repository:
[`src/assist/`](https://github.com/SohamWalam11/ros2-safety-lint/tree/main/crates/ros2-safety-lint/src/assist)
([`learned.rs`](https://github.com/SohamWalam11/ros2-safety-lint/blob/main/crates/ros2-safety-lint/src/assist/learned.rs),
[`lexical.rs`](https://github.com/SohamWalam11/ros2-safety-lint/blob/main/crates/ros2-safety-lint/src/assist/lexical.rs),
[`util.rs`](https://github.com/SohamWalam11/ros2-safety-lint/blob/main/crates/ros2-safety-lint/src/util.rs)).

Features (per option): edit and transposition distance, character 2- and 3-gram overlap, word
overlap after ROS abbreviation expansion (`vel`→`velocity`, `freq`→`frequency`, …),
prefix/suffix match, abbreviation (subsequence and word-prefix) coverage, length and word-count
ratios, and the option's rank and margin under lexical similarity.

## Training data

- **Synthetic, from real names:** the parameter names and executables that upstream ROS 2
  packages actually declare, taken from rosfix's
  [knowledge packs](https://huggingface.co/datasets/SohamW11/rosfix-knowledge-packs) (Nav2,
  ros2_controllers, ros2_control, slam_toolbox, robot_localization, …; Humble, Jazzy and Kilted
  branches). They're corrupted the way people get them wrong: typos, double typos,
  abbreviations, abbreviation+typo, dropped or swapped words, camelCase, plurals.
- **"Answer absent" cases:** the same corruptions with the intended name removed from the
  options, plus unrelated names. Every pick there is wrong, and they teach the model to abstain.
- **Split by package and name:** cases are assigned to 5 folds by package; a run tests on one
  fold, calibrates on the next and trains on the rest. A held-out case is dropped if its name
  also occurs in training (packages share many names, e.g. Nav2 base-class parameters). The
  shipped model is fold 0: 5,619 training, 1,370 calibration and 1,791 held-out cases (4,671
  more held-out cases shared a name with training and were excluded).

Trained in plain Rust (`rosfix-train`, 40 epochs of Adam on a laptop CPU, deterministic for a
given seed). Calibration then lowers all scores until the model makes no more wrong edits than
the lexical ranker on the calibration fold. The weights are written only if, summed over all 5
folds, the model is right more often than the lexical ranker *and* makes no more wrong edits.

## Results (held-out packages and names)

Packs rebuilt and model retrained on 24 November 2026. Shipped model (fold 0), correct /
abstained / wrong:

| | n | lexical ranker | **this model** |
|---|---:|---|---|
| answer offered | 1,062 | 575 / 482 / 5 | **738** / 322 / **2** |
| answer absent | 729 | — / 704 / 25 | — / **724** / **5** |
| **all** | 1,791 | 575 / 1,186 / 30 | **738** / 1,046 / **7** |

By corruption type (answer offered):

| corruption | n | lexical | this model |
|---|---:|---|---|
| typo | 143 | 69 / 74 / 0 | **133** / 10 / 0 |
| double typo | 139 | 16 / 123 / 0 | **78** / 61 / 0 |
| abbreviation | 148 | 67 / 80 / 1 | **110** / 38 / 0 |
| abbreviation + typo | 138 | 16 / 121 / 1 | **34** / 102 / 2 |
| plural | 162 | 126 / 36 / 0 | **151** / 11 / 0 |
| swapped words | 127 | 113 / 14 / 0 | **121** / 6 / 0 |
| camelCase | 106 | **105** / 1 / 0 | 102 / 4 / 0 |
| dropped word | 99 | **63** / 33 / 3 | 9 / 90 / 0 |

5-fold cross-validation (share of all held-out cases):

| fold | n | lexical correct | learned correct | lexical wrong | learned wrong |
|---|---:|---:|---:|---:|---:|
| 0 | 1,791 | 32.1% | **41.2%** | 1.68% | **0.39%** |
| 1 | 730 | 14.7% | **39.0%** | **2.05%** | 2.88% |
| 2 | 140 | 25.7% | **52.9%** | **2.14%** | 5.71% |
| 3 | 38 | 31.6% | **57.9%** | 5.26% | 5.26% |
| 4 | 306 | 23.2% | **58.5%** | **2.94%** | 3.27% |
| mean ± sd | | 25.5 ± 7.1% | **49.9 ± 9.2%** | **2.8 ± 1.4%** | 3.5 ± 2.1% |
| pooled | 3,005 | 26.7% | **43.2%** | 1.96% | **1.60%** |

The model is correct more often than lexical ranking in every fold. It makes fewer wrong edits
overall (48 vs. 59 pooled), but more in folds 1, 2 and 4; folds 2 and 3 are small. It is weaker
on dropped words, where it abstains instead. Abstaining is always safe, because the finding is
still reported, and every pick is verified by re-analysis.

On the rosfix mutation benchmark (faults injected at real sites in 12 ROS 2 repositories) it
fixed all 28 misspelled executable names (lexical: 12) and all 3 abbreviated parameter keys
(lexical: 3). Both rankers abstained on all 20 misspelled parameter keys.

## Limitations

The corruptions are synthetic. The accuracy above measures these *kinds* of mistakes, not
recall on bugs in the wild. The model ranks only the options rosfix offers, so it can't fix a
value when the intended name isn't among them (it abstains instead).

## Use

```bash
cargo install rosfix         # Rust 1.88+; PyPI wheels follow with v0.2.3
rosfix fix --assist          # preview assisted fixes
```

Reproduce the weights: `cargo run --release -p ros2-safety-mutate --bin rosfix-train` in the
source repository.

## Citation

```bibtex
@misc{walam2026rosfix,
  title  = {Cross-Language Static Analysis for {ROS} 2: Detecting Shadow Configuration Drift},
  author = {Walam, Soham Kishor},
  year   = {2026},
  note   = {rosfix, \url{https://github.com/SohamWalam11/ros2-safety-lint}}
}
```
