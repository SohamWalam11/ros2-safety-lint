# Training the built-in fix ranker

`rosfix --assist` must sometimes **pick one option** for a finding that has no deterministic
fix (e.g. `max_lin_vel` → which declared parameter?). It does so with a learned ranker
(`crates/ros2-safety-lint/src/assist/learned.rs`):

- 16 string features per (wrong text, option) pair: edit and transposition distance,
  character 2-/3-gram overlap, word overlap after ROS abbreviation expansion, prefix/suffix
  match, abbreviation (subsequence and word-prefix) coverage, length and word-count ratios,
  and the option's position and margin in the lexical ranking;
- a 16→16→1 MLP scores each option; a softmax over the options **plus a fixed
  "none of these" score** decides, so the ranker can abstain when no option is meant;
- 321 parameters (~1 KB), compiled into the binary from `learned_weights.rs`.

Training and evaluation are plain Rust, with no Python and no GPU. They run in about 15 s on a laptop.

## Train

```bash
cargo run --release -p ros2-safety-mutate --bin rosfix-train
# options: --data extra.jsonl (repeatable) --epochs 40 --seed 7 --per-name 3 --force
```

This regenerates `crates/ros2-safety-lint/src/assist/learned_weights.rs`; rebuild rosfix to ship it.

## Data

| Source | How | Notes |
|---|---|---|
| Synthetic (always) | built in | The real parameter names and executables of upstream nodes in the embedded knowledge packs (Nav2, ros2_controllers, ros2_control, slam_toolbox, robot_localization, …), corrupted the way people get them wrong: typos, double typos, abbreviations, abbreviation+typo, dropped or swapped words, camelCase, plurals. Options are the other names of the same node, top 8 by lexical rank, as at runtime. |
| "Answer absent" (always) | built in | The same corruptions with the intended name removed from the options, plus unrelated names. Every pick is wrong here; this teaches the ranker to abstain. |
| Mutation benchmark | `rosfix-bench <workspaces> --emit-dataset synthetic.jsonl` | Mutations at real sites in real workspaces. |
| Real fix commits | `python mine_fixes.py --repos repos.txt --out mined.jsonl` (needs only `git` and `rosfix` on PATH; no API token: the license is read from each clone's LICENSE file) | Only Apache-2.0 / MIT / BSD repositories, with provenance stored for each example. |

JSONL lines are `{"current": "...", "options": [...], "answer": <index>, "repo": "...", "op": "..."}`.

## Evaluation and the shipping gate

Cases are assigned to **5 folds by package (or repository)**. A run tests on one fold,
calibrates on the next and trains on the other three. A held-out case is **dropped if its name
also occurs in training**: packages share many names (e.g. Nav2 base-class parameters), so a
split by package alone would test mostly on names the ranker has already seen. Fold 0 is the
shipped model.

1. Train listwise (cross-entropy over options + "none") with Adam.
2. Calibrate on the calibration fold: lower all scores (raise the relative
   attractiveness of "none") until the learned ranker makes no more wrong edits than the
   lexical ranker.
3. Report fold 0's held-out results per corruption type, and all 5 folds (mean ± sd), against
   the lexical ranker with the runtime's abstention rule.
4. **Write the weights only if**, summed over all folds, the learned ranker is right more often
   than the lexical one **and** makes no more wrong edits (counting picks on "answer absent"
   cases as wrong). `--force` overrides.

Current result (24 Nov 2026, [`paper/data/rosfix_train_2026-11-24.txt`](../paper/data/rosfix_train_2026-11-24.txt)):
fold 0 is correct on 738 of 1,062 answer-offered cases (lexical 575) with 7 wrong edits in
1,791 cases (lexical 30). Pooled over the folds it makes 48 wrong edits vs. 59, but more than
lexical ranking in folds 1, 2 and 4.

Wrong picks are the number to drive down: abstaining is always safe, because the finding is still
reported. Every pick is also verified by re-analysis, and assisted fixes are written only
with `--fix --write --accept-assist`.

Limitations: the synthetic corruptions are designed by us, so held-out accuracy measures
how well the ranker handles *these kinds* of mistakes, not a recall estimate for real-world
bugs. Adding mined real fixes (`--data mined.jsonl`) is the way to close that gap.
