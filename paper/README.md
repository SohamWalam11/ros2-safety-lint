# Paper sources

| File | What it is |
|---|---|
| `rosfix.tex` + `references.bib` | **Main paper.** The author's draft "Cross-Language Static Analysis for ROS 2: Detecting Shadow Configuration Drift", revised for clarity: plain academic prose, one integrated structure, results in tables. Every number and citation from the draft is unchanged. |
| `rosfix_before_humanize.tex` | The previous version (draft text plus `% ADDED:` blocks), kept for comparison. |
| `rosfix_rewrite.tex` + `references_rewrite.bib` | Alternative full rewrite that uses only numbers re-measured on the current code. |

## Build

```bash
pdflatex rosfix && bibtex rosfix && pdflatex rosfix && pdflatex rosfix
# or upload rosfix.tex + references.bib to Overleaf (IEEEtran is built in)
```

## Structure of `rosfix.tex`

- Sections V-A to V-D (dataset, accuracy, shadow drift, performance) and the first part of the
  TurtleBot3 case study report the draft's measurements of an earlier rosfix version, unchanged.
- Sections V-E onwards were measured on the current code (Nov 2026): the 12 repositories with the
  current implementation (Table: time, memory, findings), a manual review of all 96 findings, the
  mutation benchmark, the repair-assistant results, installed packages and coverage.
  They are reproducible with `rosfix`, `rosfix-bench` and `rosfix-train`.
- Re-measured on 24 Nov 2026 after rebuilding the knowledge packs and rosdep keys: knowledge-pack
  table, mutation benchmark (161/161), repair assistant (held-out and benchmark tables), the
  agent and rule counts (18 agents, 93 rules), and the two pack defects the rebuild exposed
  (system-default QoS, AMCL's `add_parameter`). Raw outputs are in `data/`:
  `mutation_bench_2026-11-24.md`, `mutation_bench_revisions.txt` (repository revisions) and
  `rosfix_train_2026-11-24.txt`. Refresh the counts with `rosfix rules` before submission.

## Notes for the author

- The per-repository file counts in the draft sum to 39,117, while the total is given as 38,917. A
  `% CHECK:` comment marks this in Table II.
- `references.bib` keeps references [1]–[3] exactly as in the draft; comments next to [2] and [3]
  record the publisher records found during checking (LiSA4ROS2: IROS 2024; HAROS: IROS 2016 /
  RoSE 2021) in case you want to update them.
- Please keep the data behind the draft's numbers (raw outputs, the n = 312 labelled sample, the code
  version used) with the paper; reviewers may ask for it.
