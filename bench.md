# Benchmarks

Reproduce with:

```bash
cargo build --release -p rosfix -p ros2-safety-mutate
git submodule update --init --depth 1
./target/release/rosfix-bench crates/ros2-safety-mutate/tests/fixtures/real_world_datasets/{demos,sros2,turtlebot3,ros2_control,navigation2} \
    --max-per-op 10 --out bench_results.md
```

## Method

- **Clean run**: every finding on the unmodified repositories is listed per rule. They are
  *not* assumed to be false positives; they are labelled by hand.
- **Mutation run**: one realistic fault is injected at a time, in memory, at a site found in
  the clean analysis (a parameter the node really declares, an executable a launch file
  really starts, …). A mutation is detected only if the expected rule fires at the mutated
  line (dependency removal: a finding naming the removed dependency and package).
- Removed dependencies that are still provided through another declared workspace package
  are not injected (rosfix intentionally accepts metapackage dependencies).

## Results (2026-11, five repositories)

| Mutation | Rule | Injected | Detected |
|---|---|---:|---:|
| parameter key typo | P003 | 10 | 10 |
| double written as integer | P004 | 7 | 7 |
| `ros__parameters` misspelled | P002 | 33 | 33 |
| executable name typo | L001 | 21 | 21 |
| dependency removed from package.xml | L002 | 16 | 16 |
| Nav2 plugin type typo | N002 | 20 | 20 |
| **all** | | **107** | **107** |

Clean-run findings: E002 ×4, L002 ×42, S002 ×10 — all reviewed; see README.

## Limits

Single-fault synthetic mutations from operators written alongside the tool. They show that
each check works on real code layouts; they do not estimate recall on bugs found in the
wild. Executor, QoS, ros2_control and topic rules had no mutation sites in this corpus.
Planned: a dataset of real historical misconfiguration fixes mined from ROS 2 repositories.
