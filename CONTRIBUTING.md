# Contributing to rosfix

## Build and test

```bash
cargo test --workspace          # unit tests + CLI end-to-end tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

On machines with little free memory, add `-j 1` (and `-- --test-threads=1` for tests).

## How rosfix is organised

```
workspace::index   ──►  facts (per file)  ──►  resolve::build  ──►  launch graph
  extract/*                                                          │
                                                                     ▼
                             agents/* (one per domain, run sequentially)
                                                                     │
                                       engine::postprocess (config, suppressions, dedupe)
                                                                     │
                                         report (text / JSON / SARIF)    fix (verify → diff → write)
```

| Module | Responsibility |
|---|---|
| `extract/` | One extractor per file type: `package.xml`, CMake, `setup.py`/`setup.cfg`, launch (XML / YAML / Python static interpreter), parameter YAML (with exact spans), C++ (tree-sitter), Python (RustPython), URDF + xacro, SROS2, RViz. |
| `facts.rs` | The typed fact model every extractor produces. Unknown things are represented explicitly, never guessed. |
| `resolve.rs` | Launch graph: namespaces, remaps, launch arguments, includes, parameter-file binding (rcl key matching), launch node → source linking. |
| `agents/` | `params`, `topics`, `executor`, `control`, `packaging`, `simtime`, `qos`, `nav2`, `security`. |
| `catalog.rs` | Known controllers, Nav2 plugins/parameters, QoS of common nodes. Used only to detect near-misses. |
| `fix.rs` | Applies `Safe` fixes in memory, re-runs the analysis, keeps only fixes that remove their finding without adding errors, then writes atomically. |

## Adding a rule

1. Register it in `rules.rs` (stable ID, agent, default severity, summary and explanation).
2. Implement it in the owning agent. Emit `Diagnostic::new(ID, loc, message)` with a precise location;
   add `.related(...)`, `.help(...)` and, only if the edit is local and mechanical, `.safe_fix(...)`.
3. Stay silent when facts are incomplete (`partial`, `params_open`, unresolved names). A missed
   finding is acceptable; a false one costs user trust.
4. Add a test in the agent module using `testutil::analyze_files`, covering a positive *and* a negative case.
5. Run `rosfix-bench` on real workspaces and check the rule produces no new findings on clean code
   you cannot justify.

## Benchmark

```bash
cargo build --release -p rosfix -p ros2-safety-mutate
git submodule update --init --depth 1        # real-world datasets
./target/release/rosfix-bench crates/ros2-safety-mutate/tests/fixtures/real_world_datasets/* --out bench_results.md
```
