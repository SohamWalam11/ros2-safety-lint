# rosfix

[![CI](https://github.com/SohamWalam11/ros2-safety-lint/actions/workflows/ci.yml/badge.svg)](https://github.com/SohamWalam11/ros2-safety-lint/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/rosfix.svg)](https://crates.io/crates/rosfix)
[![PyPI](https://img.shields.io/pypi/v/rosfix.svg)](https://pypi.org/project/rosfix/)
[![Hugging Face](https://img.shields.io/badge/%F0%9F%A4%97-rosfix--assist-yellow)](https://huggingface.co/SohamW11/rosfix-assist)
[![License: MIT/Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue.svg)](#license)

**A static checker for the ROS 2 misconfigurations that fail silently.**

ROS 2 configuration is spread over launch files, parameter YAML, C++/Python nodes,
`package.xml`/CMake/`setup.py`, URDF/xacro, controller, Nav2, MoveIt and ros_gz_bridge configs.
Many mistakes in that spread are never reported at runtime: a parameter under the wrong node key
is simply dropped, a remap that matches nothing does nothing, a RELIABLE subscriber never hears a
BEST_EFFORT publisher.

rosfix reads the whole workspace, **resolves the launch graph** the way ROS 2 would (namespaces,
remaps, launch arguments, includes, parameter-file matching), links every launched node to its
source code, and reports what will not work, before you run anything.

- **No ROS installation needed.** Runs on Linux, macOS and Windows; uses your installed ROS 2
  when a shell is sourced.
- **Small and frugal.** Single-threaded by default, typically 10–50 MB of RAM, no GPU.
- **Silent when unsure.** Facts it cannot resolve statically turn dependent checks off instead
  of producing guesses.
- **Verified fixes.** Every automatic fix is re-checked against the whole workspace before a
  file is written.

## Install

| Channel | Command |
|---|---|
| PyPI (adds `ros2 rosfix`) | `pip install rosfix` |
| crates.io (Rust 1.88+) | `cargo install rosfix` |
| Docker | `docker run --rm -v "$PWD:/ws" ghcr.io/sohamwalam11/rosfix:jazzy-lite /ws` |
| Prebuilt binaries | [GitHub releases](https://github.com/SohamWalam11/ros2-safety-lint/releases) (Linux x86_64/aarch64, macOS x86_64/arm64, Windows x86_64; about 15 MB) |

Docker images exist for `humble`, `jazzy` and `kilted` (`ghcr.io/sohamwalam11/rosfix:<distro>-lite`,
or `:v0.2.3-<distro>-lite` for a fixed version). Every channel ships the same binary, with the fix
ranker and knowledge packs built in. More in the
[install guide](https://github.com/SohamWalam11/ros2-safety-lint/blob/main/crates/ros2-safety-lint/docs/install.md).

## Quick start

```bash
cd ~/ros2_ws
rosfix                        # analyse the workspace in the current directory
rosfix --distro jazzy         # also check nodes from installed packages (Nav2, ros2_control, …)
rosfix fix                    # preview verified automatic fixes as a diff
rosfix fix --write            # apply them
```

Typical output:

```
warning[P003]: `deadzon` is not declared by node `/joy_node` … help: Did you mean `deadzone`?
```

Exit codes: `0` clean, `1` findings at or above `--fail-on` (default `warning`), `2` usage or tool
error.

## What it checks

93 rules in 18 agents (17 domain agents, plus `core`, which reports files it could not parse):

| Agent | Rules | Example |
|---|---|---|
| **params** | P001–P008 | YAML key `controller_server` while the node runs as `/robot1/controller_server`; `max_velocty` (typo) not declared by the node; `max_vel: 1` for a `double` (crashes at startup); missing `ros__parameters`; value outside a declared range; parameter renamed between distributions; two parameters that contradict each other. |
| **topics** | T001–T005 | Subscriber on `/scan` while the publisher is on `/robot1/scan`; remap of a name the node never uses; one topic with two message types. |
| **executor** | E001–E002 | Synchronous service call / future wait / spin inside a callback that shares the callback group (or a single-threaded executor) with the response, which deadlocks; sleep in a callback. |
| **control** | C001–C010 | Controller joint not in any `<ros2_control>` block; velocity controller on a position-only joint; controller type typo; `<ros2_control>` joint missing from the URDF; spawned controller never defined; `update_rate` the controller manager cannot run. |
| **packaging** | L001–L012 | Launch file starts an executable the package does not build/install; dependency used but not in `package.xml`; `setup.cfg` without `install_scripts`; `launch/` never installed; dependency `rosdep` cannot resolve; `install()` of a path that does not exist; Python subpackage missing from `setup.py`. |
| **launch** | A001–A003 | `$(var x)` never declared; declared argument never used; two nodes with the same fully qualified name. |
| **simtime** | S001–S003 | Some nodes on simulated time, others on wall time; simulator launched but a node not using `/clock`. |
| **qos** | Q001–Q005 | RELIABLE subscription on a BEST_EFFORT publisher (incl. RViz displays); TRANSIENT_LOCAL subscription on a VOLATILE publisher; `qos_overrides` in YAML for an endpoint that does not accept overrides (silently ignored). |
| **nav2** | N001–N006, N008–N010 | Plugin listed without a section; plugin type typo; frame not in the URDF; degenerate footprint; inflation radius smaller than the robot; plugin renamed between distributions; unknown behavior-tree node. |
| **lifecycle** | M001–M002 | Lifecycle node no manager activates; manager lists a node that does not exist. |
| **tf** | F001–F005 | Frame with two parents; bad `static_transform_publisher` arguments; leading slash in a lookup; cycle in the static tree. |
| **network** | D001–D005 | Invalid `ROS_DOMAIN_ID`; unknown `RMW_IMPLEMENTATION`; missing DDS profile file; deprecated `ROS_LOCALHOST_ONLY`; invalid Zenoh setting. |
| **security** | X001–X006 | Unauthenticated participants allowed; `<default>ALLOW`; wildcard grants; expired permissions; unknown enclave; node publishing a topic its SROS2 policy denies. |
| **moveit** | V001–V005 | `kinematics.yaml` group not in the SRDF; integer joint limit; SRDF names a joint the URDF lacks; MoveIt controller no ros2_control config defines. |
| **urdf** | U001–U004 | Element urdfdom rejects; impossible inertia; missing `package://` mesh; empty joint-limit range. |
| **gzbridge** | G001–G005 | ROS/Gazebo type pair `ros_gz_bridge` cannot convert; malformed bridge argument or YAML entry; one-way bridge pointing the wrong way. |
| **ros1** | O001–O003 | ROS 1 launch file, `ros/ros.h` include or catkin package left in a ROS 2 workspace. |

`rosfix rules` lists all rules; `rosfix explain P001` explains one.

**When rosfix cannot know, it stays silent.** Facts that cannot be resolved statically (dynamic
launch code, parameters declared by plugins, external packages) are tracked explicitly and turn
the dependent checks off instead of guessing. `rosfix graph` shows exactly what was understood.

## Works with your installed ROS

In a sourced ROS 2 shell, rosfix reads the installed environment (`AMENT_PREFIX_PATH`): installed
packages, their executables, pluginlib plugins and rclcpp components, and the launch and parameter
files of packages your launch files include.

For nodes from installed packages (Nav2, ros2_controllers, robot_state_publisher, slam_toolbox,
robot_localization, …) it uses a built-in **knowledge pack** per distribution (Humble, Jazzy,
Kilted; Rolling uses the newest), extracted from their source. So `controller_frequency: 20` for
Nav2's controller server is reported as a startup crash without Nav2's source on your machine.

- The distribution comes from `ROS_DISTRO`, `--distro`, or `distro = "jazzy"` in `rosfix.toml`.
  Without one, the packs are not used, because parameter names differ between distributions.
- `--ros-prefix` points at another install; `--no-ros` ignores the installed environment.

Each pack records every node's declared parameters (with type and default) and its topic
endpoints with the QoS its source requests. An endpoint that asks for the system-default profile
is kept as *unknown*, since the middleware decides it. Each distribution also embeds the keys
`rosdep` can resolve (4,590 Humble, 4,696 Jazzy, 3,990 Kilted). The packs are published as a
[dataset on Hugging Face](https://huggingface.co/datasets/SohamW11/rosfix-knowledge-packs).

## Usage

```bash
rosfix check ~/ros2_ws/src                     # analyse a directory
rosfix check --format sarif -o rosfix.sarif    # for GitHub code scanning
rosfix check --format json                     # stable, versioned JSON
rosfix check --format rdjson | reviewdog -f=rdjson -reporter=github-pr-review  # fixes as PR suggestions
rosfix graph --format mermaid                  # resolved launch graph (text, json, dot, mermaid)
rosfix graph --format callbacks                # static callback graph: executors, timers, subscriptions
rosfix migrate --to kilted                     # what breaks on another ROS distribution
rosfix doctor                                  # check this shell's ROS 2 / middleware setup (DDS, Zenoh)
rosfix triage launch.log                       # explain errors in a failed launch log
rosfix init                                    # write rosfix.toml and a GitHub workflow
rosfix status                                  # what each agent checked, found, and what limits it
rosfix report -o report.html                   # one HTML page: findings, agents, launch graph
rosfix shell src                               # interactive, with history and Tab completion
rosfix docs                                    # the full manual, built into the binary
```

The original flags (`rosfix --fix --write`, `--print-graph`, `--list-rules`, …) keep working.
The full command reference is in the
[manual](https://github.com/SohamWalam11/ros2-safety-lint/tree/main/crates/ros2-safety-lint/docs)
and in `rosfix docs`.

### Editors

`rosfix lsp` is a language server: findings as you type, fixes as quick fixes. A
[VS Code extension](https://github.com/SohamWalam11/ros2-safety-lint/tree/main/editors/vscode)
wraps it.

### GitHub Actions

```yaml
- uses: actions/checkout@v4
- uses: SohamWalam11/ros2-safety-lint@v0.2.3
  with:
    path: src
    distro: jazzy
    fail-on: error
    pr-suggestions: "true"   # verified fixes as one-click suggestions on pull requests
```

The action downloads the checksum-verified release binary and uploads SARIF to code scanning. To
install from crates.io instead, copy the
[workflow template](https://github.com/SohamWalam11/ros2-safety-lint/blob/main/.github/workflows/ros2-safety-lint-template.yml).

### Configuration (`rosfix.toml`)

```toml
ignore = ["third_party/**"]
test_paths = ["**/test/**"]           # executor rules skip these
external_topics = ["/scan"]           # provided outside this workspace
fail_on = "error"
distro = "jazzy"                      # enables the knowledge pack without a sourced shell
online = false                        # allow --online lookups by default
# searxng_url = "https://searx.example.org"

[rules]
P003 = "off"
Q004 = "warning"
```

Unknown keys and rule IDs are errors, not silently ignored. Suppress one finding with a comment on
the same or previous line: `# rosfix: ignore[P003]` (also `//` and `<!-- -->`). To adopt rosfix on
an existing codebase, run `--write-baseline rosfix-baseline.json` once and
`--baseline rosfix-baseline.json` afterwards: only new findings are reported.

## Fixes

### Automatic fixes

Only local, mechanical fixes are automatic (e.g. `1` → `1.0`, renaming a misspelled key, adding a
`<depend>`, installing `launch/`). Every candidate fix is applied **in memory**, the whole
workspace is re-analysed, and the fix is kept only if its finding disappears and no new error
appears. Files are then written atomically. Everything else is shown as a suggestion.

### Build check (`--verify-build`)

`--fix --verify-build` copies the workspace to a temporary directory and builds it before and
after the fixes, in a network-less `ros:<distro>` container when Docker is available, otherwise
with your local `colcon`. Fixes that break a build that worked are not written.

### Fix assistant (`--assist`)

Some findings have no mechanical fix, e.g. a parameter written as `max_lin_vel` when the node
declares `max_linear_velocity`. Each such finding carries the closed set of valid values rosfix
knows (declared parameters, `<ros2_control>` joints, URDF frames, executables, enclaves).
`--assist` picks one of them with a **built-in learned ranker**: 16 string features per option
(edit distances, n-gram overlap, abbreviation and word-prefix coverage, …) scored by a tiny neural
network. It has 289 weights and 32 normalisation constants (about 1.3 KB), compiled into the
binary, and needs no model files, ML runtime, GPU or network. It scores a finding in microseconds.

It is evaluated on held-out packages **and** names: a test case is dropped if its name also occurs
in training, because packages share many names. On the shipped model's held-out cases it picks the
right value in 69.5% of cases against 54.1% for plain lexical ranking, and makes 7 wrong picks in
1,791 cases against 30. It also abstains when none of the options is the intended one (for
example a remap of a name the node never uses).

| held-out (shipped model) | cases | lexical: correct / abstain / wrong | learned: correct / abstain / wrong |
|---|---:|---|---|
| right answer offered | 1,062 | 575 / 482 / 5 | **738** / 322 / **2** |
| right answer absent (any pick is wrong) | 729 | 0 / 704 / 25 | 0 / **724** / **5** |

Over 5 package folds (3,005 held-out cases) it is correct 43.2% of the time against 26.7% and
makes 48 wrong picks against 59. It is correct more often in every fold, but in 3 of the 5 folds
it makes slightly more wrong picks than lexical ranking. On the mutation benchmark below it fixed
all 28 misspelled executable names (lexical: 12).

The assistant only chooses among real options, never invents text, and abstains unless there is a
clear winner. Every choice is verified by re-analysis. Because verification cannot prove intent,
assisted fixes are written only with `--fix --write --accept-assist`.

Weights and model card: [SohamW11/rosfix-assist](https://huggingface.co/SohamW11/rosfix-assist).
Training and evaluation:
[training/README.md](https://github.com/SohamWalam11/ros2-safety-lint/blob/main/training/README.md).

#### Larger assist models (in development)

`--model` selects the ranker: `builtin` (default), a downloadable tier, or a `.rfm` file path.
Tiers also see the rule and the source lines around the problem, not just the two names. They are
the same int8 transformer in different sizes, run on any CPU, need no GPU, network or Python, and
give bit-identical results on every machine.

| tier | stored numbers | file | per candidate, 64-token input* |
|---|---:|---:|---:|
| builtin | 321 | in the binary | < 0.01 ms |
| nano | 1.46 M | 1.6 MB | 5 ms |
| mini | 5.32 M | 5.5 MB | 25 ms |
| base | 23.3 M | 23.6 MB | 85 ms |

\* Median of 5 trials, one thread, on a 15 W laptop CPU (Intel Core 5 120U)
([raw data](https://github.com/SohamWalam11/ros2-safety-lint/blob/main/paper/data/tier_runtime_trials_2026-11-24.txt)).

**No tier has been released yet.** A tier name only loads a checksum-verified released file. Until
a tier beats the built-in ranker with no more wrong picks in every evaluation fold, rosfix uses
the built-in ranker.

Training data so far, all built in plain Rust and reproducible byte for byte: an 8,192-token
byte-level BPE tokenizer learned from the config files of 830 public ROS 2 repositories (2.71
bytes per token on 74 held-out repositories), and the built-in ranker's 16,456 cases with rule and
source context (65 tokens on average). Details:
[crates/rosfix-model](https://github.com/SohamWalam11/ros2-safety-lint/tree/main/crates/rosfix-model).

## Online references (`--online`)

Off by default. With `--online` (or `online = true`), rosfix attaches links to related Robotics
Stack Exchange questions and GitHub issues (and your own SearXNG instance, if `searxng_url` is
set).

Queries are built **only from the rule that fired**: no file content, path, node or package name
leaves your machine (`--show-queries` prints them). The lookup runs in a separate helper process
restricted to an allowlist of HTTPS hosts, with no redirects, short timeouts and size limits.
Results are cached locally.

**Beyond the embedded packs.** With `--online` and a known distribution, rosfix also covers
launched packages outside the embedded packs. It looks each one up in the public rosdistro index
of the distribution, downloads the source of its repository at the distribution's branch, runs
its own extractors over that source once, and stores the result as a *local pack*
(`<cache>/rosfix/packs/<distro>/`, or `$ROSFIX_CACHE_DIR`). Later runs use the local packs without
network access.

For example, a typo in a `joy_node` parameter file is found only after `joy` has been fetched:

```
$ rosfix . --distro jazzy --online --show-queries
rosfix: online request: https://raw.githubusercontent.com/ros/rosdistro/master/jazzy/distribution.yaml
rosfix: online request: https://codeload.github.com/ros-drivers/joystick_drivers/tar.gz/ros2
rosfix: online: built a local pack from ros-drivers/joystick_drivers@ros2 (3 package(s) with nodes)
rosfix: online: now covered: joy
warning[P003]: `deadzon` is not declared by node `/joy_node` … help: Did you mean `deadzone`?
```

Package names are matched against the downloaded index locally, so names of private packages
never leave the machine. The requests do show GitHub which public repositories you use. Archives
are unpacked with path checks and size limits. Only source, build and manifest files are kept,
nothing is executed, and the unpacked sources are deleted after the pack is built. A local pack
only adds packages; it never replaces what an embedded pack says. Delete the directory to drop
the local packs.

## Evaluation

Measured on 24 November 2026 on shallow clones of each repository's default branch (release
build, one thread, median of three runs after one warm-up, peak working set). Raw outputs and the
exact revisions are in
[paper/data](https://github.com/SohamWalam11/ros2-safety-lint/tree/main/paper/data).

| Repository | Files parsed | Wall time | Peak RAM | Findings |
|---|---:|---:|---:|---|
| ros2/demos | 145 | 0.15 s | 18 MB | E002 ×2, L002 ×8 |
| ros2/sros2 | 20 | 0.03 s | 11 MB | none |
| ROBOTIS-GIT/turtlebot3 | 63 | 0.07 s | 16 MB | E002 ×2, L002 ×8 |
| ros-controls/ros2_control | 124 | 0.60 s | 25 MB | none |
| ros-navigation/navigation2 | 778 | 2.08 s | 51 MB | L002 ×49, S002 ×9, A002 ×2, N009, P005, T003 |

The L002 findings are missing `package.xml` dependencies (e.g. `turtlebot3_bringup` starts
`rclcpp_components`, `camera_ros` and `image_view` without declaring them); E002 are sleeps inside
executor callbacks; S002 flags nodes in Nav2 example launch files that run beside a simulator
without `use_sim_time` (low impact, but accurate).

**Mutation benchmark.** Realistic misconfigurations are injected one at a time, in memory, at real
sites in 12 repositories (PX4-Autopilot, autoware, moveit2, navigation2, rmf_ros2, ros2_control,
rmw_fastrtps, demos, system_tests, turtlebot3, sros2, rmw_cyclonedds; at most 20 sites per
operator and repository). A mutation counts as detected only if the expected rule fires at the
mutated line.

| Mutation | Injected | Detected |
|---|---:|---:|
| `ros__parameters` misspelled (P002) | 50 | 50 |
| Nav2 plugin type typo (N002) | 40 | 40 |
| executable name typo in a launch file (L001) | 28 | 28 |
| parameter key typo (P003) | 20 | 20 |
| runtime dependency removed from package.xml (L002) | 16 | 16 |
| double written as integer (P004) | 4 | 4 |
| parameter key abbreviated (P003) | 3 | 3 |
| **total** | **161** | **161** |

These are synthetic, single-fault mutations designed alongside the tool, not a recall estimate for
bugs in the wild. The executor, QoS, ros2_control and topic agents are covered by unit tests but
had no mutation sites in this corpus.

**Hand review of the other findings on the 12 repositories.** P003: the 3 in rmf_ros2 are real
(undeclared `subns`, `verbose`, `run_composition_command`). O001/O002: all 31 ROS 1 leftovers are
accurate; the 20 moveit2 sources with `ros/ros.h`/`rospy` are in no CMake target (dead code).
U002: MoveIt's PRBT test robot really gives `prbt_flange` (revolute joint) zero inertia.

**Field study.** rosfix was run on 826 rosdistro Jazzy repositories, twice at the same commits
(20 and 25 November 2026). Findings went from 3,307 to 4,295, mostly from rules added in between;
a hand check of about 40 new findings found them correct, and no triaged true positive was lost
([comparison](https://github.com/SohamWalam11/ros2-safety-lint/blob/main/research/data/s2_rerun_2026-11-25_fix.md)).

The full evaluation (precision on a hand-labelled sample, cross-file drift, incremental
re-analysis, fix verification on real repositories) is in the
[paper](https://github.com/SohamWalam11/ros2-safety-lint/tree/main/paper).

## Milestones

Each milestone is also described, with its measurements, in the paper.

**25 November 2026**
- **Released v0.2.3** on [crates.io](https://crates.io/crates/rosfix),
  [PyPI](https://pypi.org/project/rosfix/) (wheels for Linux x86_64/aarch64, macOS x86_64/arm64
  and Windows), GitHub Container Registry (Humble, Jazzy, Kilted images) and as prebuilt
  binaries, all from one tagged workflow with build provenance attestations.
- **Assist tiers: runtime and trainer.** An int8 transformer that scores the same closed
  candidate set as the built-in ranker, with the rule and the surrounding source as extra input,
  and gives bit-identical scores on every machine. `--model` selects it; only checksum-verified
  released files load. No tier is released yet.
- **Training is bit-identical across operating systems.** Weight initialisation used the
  platform's `ln`/`cos`, so one seed trained different models on Linux, Windows and macOS (CI
  caught int8 export changing 34 of 300 decisions on Linux). It now uses exact operations only;
  300/300 decisions survive int8 export on all three.
- **CI green on Linux, Windows and macOS**, with `cargo fmt` and `clippy -D warnings` enforced and
  the minimum Rust version raised to 1.88 (what the dependencies require).
- **Field study re-run on 826 rosdistro Jazzy repositories** at the same commits as on 20
  November. Findings went from 3,307 to 4,295, mostly from rules added since (L009 292, O002 96,
  N008 82, O001 55, U002 53) and 373 more L002 for launch files that import `launch_ros` without
  declaring it. A hand check of about 40 new findings found them correct, except U004 firing on
  URDF test fixtures that are broken on purpose. The re-run exposed one regression, now fixed:
  `if=`/`unless=` were not evaluated, so a confirmed S001 in rmf_demos was lost. rosfix now
  evaluates conditions with the arguments the launch graph passes, and no confirmed true positive
  is lost.

**24 November 2026**
- **93 rules in 18 agents**, including the new A003 (duplicate node name), F005 (tf cycle), L011
  (`install()` of a missing path) and L012 (Python subpackage not installed).
- **Knowledge packs rebuilt** from the Humble, Jazzy and Kilted branches. Two defects fixed:
  system-default QoS endpoints are now *unknown* (4, 6 and 8 endpoints were treated as the ROS
  default), and Nav2 AMCL's `add_parameter` declarations are now extracted (0/7/8 parameters
  became 45/53/54). Before, every AMCL parameter was reported as undeclared when Nav2 was
  installed from binaries.
- **Fewer false positives for undeclared parameters (P003): 46 → 3** on the 12 benchmark
  repositories. Parameter libraries generated in CMake subdirectories and in workspace
  dependencies (MoveIt's smoothing filters) now count as declarations. The 3 that remain are real.
- **Fix ranker retrained and evaluated honestly**: held-out cases must come from unseen packages
  *and* unseen names, over 5 folds. Shipped model: 69.5% correct vs. 54.1% lexical, 7 wrong picks
  vs. 30.
- **Mutation benchmark 161/161** on 12 repositories at current revisions, and a hand review of
  every finding of the new rules on them (all accurate).
- **Published on Hugging Face**: the fix ranker and the knowledge packs.

## Resources

| | |
|---|---|
| Fix ranker (model card, weights) | [huggingface.co/SohamW11/rosfix-assist](https://huggingface.co/SohamW11/rosfix-assist) |
| Knowledge packs (dataset) | [huggingface.co/datasets/SohamW11/rosfix-knowledge-packs](https://huggingface.co/datasets/SohamW11/rosfix-knowledge-packs) |
| Manual | [docs](https://github.com/SohamWalam11/ros2-safety-lint/tree/main/crates/ros2-safety-lint/docs), or `rosfix docs` |
| Rust API | [docs.rs/rosfix](https://docs.rs/rosfix) |
| Paper and raw data | [paper/](https://github.com/SohamWalam11/ros2-safety-lint/tree/main/paper) |
| Contributing (architecture, adding a rule) | [CONTRIBUTING.md](https://github.com/SohamWalam11/ros2-safety-lint/blob/main/CONTRIBUTING.md) |
| Issues | [github.com/SohamWalam11/ros2-safety-lint/issues](https://github.com/SohamWalam11/ros2-safety-lint/issues) |

The repository holds the `rosfix` library and CLI (`crates/ros2-safety-lint`), the benchmark,
training and pack-building tools (`crates/ros2-safety-mutate`), and the runtime and trainer for
the assist tiers (`crates/rosfix-model`).

## Citation

If you use rosfix in research, please cite:

```bibtex
@misc{walam2026rosfix,
  title  = {Cross-Language Static Analysis for {ROS} 2: Detecting Shadow Configuration Drift},
  author = {Walam, Soham Kishor},
  year   = {2026},
  note   = {rosfix, \url{https://github.com/SohamWalam11/ros2-safety-lint}}
}
```

## License

Licensed under either of
[Apache License, Version 2.0](https://github.com/SohamWalam11/ros2-safety-lint/blob/main/LICENSE-APACHE)
or [MIT License](https://github.com/SohamWalam11/ros2-safety-lint/blob/main/LICENSE-MIT) at your
option.
