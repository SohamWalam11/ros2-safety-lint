# rosfix

[![CI](https://github.com/SohamWalam11/ros2-safety-lint/actions/workflows/ci.yml/badge.svg)](https://github.com/SohamWalam11/ros2-safety-lint/actions)
[![License: MIT/Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)

**A static checker for the ROS 2 misconfigurations that fail silently.**

ROS 2 configuration is spread over launch files, parameter YAML, C++/Python nodes,
`package.xml`/CMake/`setup.py`, URDF/xacro, controller, Nav2, MoveIt and ros_gz_bridge
configs. Many mistakes in
that spread are never reported at runtime: a parameter under the wrong node key is simply
dropped, a remap that matches nothing does nothing, a RELIABLE subscriber never hears a
BEST_EFFORT publisher. rosfix reads the whole workspace, **resolves the launch graph** the
way ROS 2 would (namespaces, remaps, launch arguments, includes, parameter-file matching),
links every launched node to its source code, and reports what will not work — before you
run anything.

It is small and frugal: single-threaded by default, typically 10–50 MB of RAM, no GPU,
no ROS installation needed.

## What it checks

93 rules in 18 agents (17 domain agents, plus `core`, which reports files it could not parse):

| Agent | Rules | Example |
|---|---|---|
| **params** | P001–P008 | YAML key `controller_server` while the node runs as `/robot1/controller_server`; `max_velocty` (typo) not declared by the node; `max_vel: 1` for a `double` (crashes at startup); missing `ros__parameters`; value outside a declared range; parameter renamed between distributions; two parameters that contradict each other. |
| **topics** | T001–T005 | Subscriber on `/scan` while the publisher is on `/robot1/scan`; remap of a name the node never uses; one topic with two message types. |
| **executor** | E001–E002 | Synchronous service call / future wait / spin inside a callback that shares the callback group (or a single-threaded executor) with the response — a deadlock; sleep in a callback. |
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

`rosfix rules` prints all rules; `rosfix explain P001` explains one.

Design rule: **when rosfix cannot know, it stays silent.** Facts that could not be
resolved statically (dynamic launch code, parameters declared by plugins, external
packages) are tracked explicitly and turn the dependent checks off instead of guessing.
`rosfix --print-graph` shows exactly what was understood.

## Works with your installed ROS

In a sourced ROS 2 shell, rosfix reads the installed environment (`AMENT_PREFIX_PATH`):
installed packages, their executables, pluginlib plugins and rclcpp components, and the launch
and parameter files of packages your launch files include. For nodes from installed packages
(Nav2, ros2_controllers, robot_state_publisher, slam_toolbox, robot_localization, …) it uses a
built-in **knowledge pack** per distribution (Humble, Jazzy, Kilted; Rolling uses the newest),
extracted from their source, so e.g. `controller_frequency: 20` for Nav2's controller server is
reported as a startup crash without having Nav2's source. The distribution comes from
`ROS_DISTRO`, `--distro`, or `distro = "jazzy"` in `rosfix.toml`; without it, the packs are not
used (parameter names differ between distributions). `--ros-prefix` points at another install;
`--no-ros` ignores it.

## Install

```bash
pip install rosfix                 # prebuilt wheels: Linux x86_64/aarch64, macOS, Windows
ros2 rosfix src                    # also available as a ros2 command after pip install
docker run --rm -v "$PWD:/ws" ghcr.io/sohamwalam11/rosfix:jazzy-lite /ws
cargo install rosfix               # from source
```

Standalone single-file archives for every platform are attached to each GitHub release
(about 15 MB; the fix ranker is built in).

## Usage

```bash
rosfix                       # analyse the current workspace (same as `rosfix check`)
rosfix check ~/ros2_ws/src   # analyse a directory
rosfix check --format sarif -o rosfix.sarif   # for GitHub code scanning
rosfix check --format json   # stable, versioned JSON
rosfix fix                   # preview verified automatic fixes as a diff (nothing is written)
rosfix fix --write           # apply them
rosfix check --format rdjson | reviewdog -f=rdjson -reporter=github-pr-review  # fixes as PR suggestions
rosfix graph --format mermaid  # the resolved launch graph (text, json, dot, mermaid)
rosfix graph --format callbacks  # static callback graph: executors, timers with periods, subscriptions
rosfix migrate --to kilted   # what breaks on another ROS distribution
rosfix doctor                # check this shell's ROS 2 / middleware environment (DDS and Zenoh)
rosfix triage launch.log     # explain errors in a failed launch log with the findings
rosfix init                  # write rosfix.toml and a GitHub workflow
rosfix status                # what each agent checked, found, and what limits it
rosfix report -o report.html # one HTML page: findings, agents, launch graph
rosfix shell src             # interactive, with history and Tab completion
rosfix docs                  # the full manual, built into the binary
```

The original flags (`rosfix --fix --write`, `--print-graph`, `--list-rules`, …) keep working.

In editors, `rosfix lsp` is a language server with findings as you type and fixes as quick
fixes; [editors/vscode](editors/vscode) is a VS Code extension for it.

Exit codes: `0` clean, `1` findings at or above `--fail-on` (default `warning`), `2` usage or tool error.

### Automatic fixes

Only local, mechanical fixes are automatic (e.g. `1` → `1.0`, renaming a misspelled key,
adding a `<depend>`, installing `launch/`). Every candidate fix is applied **in memory**,
the whole workspace is re-analysed, and the fix is kept only if its finding disappears and
no new error appears; then files are written atomically. Everything else is shown as a
suggestion.

### Fix assistant (`--assist`)

Some findings have no mechanical fix, e.g. a parameter written as `max_lin_vel` when the node
declares `max_linear_velocity`. Each such finding carries the closed set of valid values rosfix
knows (declared parameters, `<ros2_control>` joints, URDF frames, executables, enclaves).
`--assist` picks one of them with a **built-in learned ranker**: 16 string features per option
(edit distances, n-gram overlap, abbreviation and word-prefix coverage, …) scored by a tiny
neural network. It has 289 weights and 32 normalisation constants (about 1.3 KB), compiled into
the binary, and needs no model files, ML runtime, GPU or network. It scores a finding in
microseconds.

It is evaluated on held-out packages **and** names: a test case is dropped if its name also
occurs in training, because packages share many names. On the shipped model's held-out cases it
picks the right value in 69.5% of cases against 54.1% for plain lexical ranking, and makes 7
wrong picks in 1,791 cases against 30. It also abstains when none of the options is the intended
one (for example a remap of a name the node never uses).

| held-out (shipped model) | cases | lexical: correct / abstain / wrong | learned: correct / abstain / wrong |
|---|---:|---|---|
| right answer offered | 1,062 | 575 / 482 / 5 | **738** / 322 / **2** |
| right answer absent (any pick is wrong) | 729 | 0 / 704 / 25 | 0 / **724** / **5** |

Over 5 package folds (3,005 held-out cases) it is correct 43.2% of the time against 26.7% and
makes 48 wrong picks against 59. It is correct more often in every fold, but in 3 of the 5 folds
it makes slightly more wrong picks than lexical ranking. That is why assisted fixes are verified
and written only on explicit request.

On the mutation benchmark below it fixed all 28 misspelled executable names (lexical: 12).
Weights and model card: [huggingface.co/SohamW11/rosfix-assist](https://huggingface.co/SohamW11/rosfix-assist).

The assistant can only choose among real options, never invent text, and abstains unless
there is a clear winner. Every choice is verified by re-analysis. Because verification
cannot prove intent, assisted fixes are written only with `--fix --write --accept-assist`.
How the ranker is trained and evaluated: [`training/README.md`](training/README.md).

#### Larger assist models (in development)

`--model` selects the ranker: `builtin` (default), a downloadable tier, or a `.rfm` file path.
Tiers also see the rule and the source lines around the problem, not just the two names. They
are the same int8 transformer in different sizes, run on any CPU, need no GPU, network or
Python, and give bit-identical results on every machine.

| tier | stored numbers | file | per candidate, 64-token input* |
|---|---:|---:|---:|
| builtin | 321 | in the binary | < 0.01 ms |
| nano | 1.46 M | 1.6 MB | 5 ms |
| mini | 5.32 M | 5.5 MB | 25 ms |
| base | 23.3 M | 23.6 MB | 85 ms |

\* Median of 5 trials, one thread, Intel Core 5 120U laptop under normal background load
([raw data](paper/data/tier_runtime_trials_2026-11-24.txt)).

**No tier has been released yet.** A tier name only loads a checksum-verified released file, and
until a tier is trained and beats the built-in ranker with no more wrong picks in every
evaluation fold, rosfix uses the built-in ranker. The runtime and trainer live in
[`crates/rosfix-model`](crates/rosfix-model); `rosfix-model-bench` reproduces the timings.

### Online references (`--online`)

Off by default. With `--online` (or `online = true`), rosfix attaches links to related
Robotics Stack Exchange questions and GitHub issues (and your own SearXNG instance, if
`searxng_url` is set). Queries are built **only from the rule that fired**: no file content,
path, node or package name leaves your machine (`--show-queries` prints them). The lookup runs
in a separate helper process restricted to an allowlist of HTTPS hosts, with no redirects,
short timeouts and size limits. Results are cached locally.

### Build check (`--verify-build`)

`--fix --verify-build` copies the workspace to a temporary directory and builds it before and
after the fixes, in a network-less `ros:<distro>` container when Docker is available,
otherwise with your local `colcon`. Fixes that break a build that worked are not written.

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

Unknown keys and rule IDs are errors, not silently ignored. Suppress one finding with a
comment on the same or previous line: `# rosfix: ignore[P003]` (also `//` and `<!-- -->`).
Adopt rosfix on an existing codebase with `--write-baseline rosfix-baseline.json` once and
`--baseline rosfix-baseline.json` afterwards: only new findings are reported.

### GitHub Actions

See [`.github/workflows/ros2-safety-lint-template.yml`](.github/workflows/ros2-safety-lint-template.yml).

## Evaluation

All numbers below were measured on 24 November 2026 with the current code, on shallow clones of
the repositories' default branches (Windows 11, release build, one thread; median of three runs
after one warm-up, peak working set). Raw outputs and the exact repository revisions are in
[`paper/data/`](paper/data).

| Repository | Files parsed | Wall time | Peak RAM | Findings |
|---|---:|---:|---:|---|
| ros2/demos | 145 | 0.15 s | 18 MB | E002 ×2, L002 ×8 |
| ros2/sros2 | 20 | 0.03 s | 11 MB | none |
| ROBOTIS-GIT/turtlebot3 | 63 | 0.07 s | 16 MB | E002 ×2, L002 ×8 |
| ros-controls/ros2_control | 124 | 0.60 s | 25 MB | none |
| ros-navigation/navigation2 | 778 | 2.08 s | 51 MB | L002 ×49, S002 ×9, A002 ×2, N009, P005, T003 |

The L002 findings are missing `package.xml` dependencies (e.g. `turtlebot3_bringup` starts
`rclcpp_components`, `camera_ros` and `image_view` without declaring them); E002 are sleeps
inside executor callbacks; S002 flags nodes in Nav2 example launch files that run beside a
simulator without `use_sim_time` (low impact, but accurate).

**Mutation benchmark** (`rosfix-bench`, in `crates/ros2-safety-mutate`): realistic
misconfigurations are injected one at a time, in memory, at real sites found in 12 repositories
(PX4-Autopilot, autoware, moveit2, navigation2, rmf_ros2, ros2_control, rmw_fastrtps, demos,
system_tests, turtlebot3, sros2, rmw_cyclonedds; at most 20 sites per operator and repository).
A mutation counts as detected only if the expected rule fires at the mutated line.

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

These numbers measure the checks on synthetic, single-fault mutations designed alongside
the tool; they are not a recall estimate for bugs in the wild. The executor, QoS,
ros2_control and topic agents are covered by unit tests but had no mutation sites in this
corpus.

**Hand review of the other findings on the 12 repositories.** P003: the 3 in rmf_ros2 are real
(undeclared `subns`, `verbose`, `run_composition_command`). O001/O002: all 31 ROS 1 leftovers are
accurate; the 20 moveit2 sources with `ros/ros.h`/`rospy` are in no CMake target (dead code).
U002: MoveIt's PRBT test robot really gives `prbt_flange` (revolute joint) zero inertia.

The full evaluation (precision on a hand-labelled sample, cross-file drift, incremental
re-analysis, fix verification on real repositories) is in the [paper](paper/rosfix.tex).

## Hugging Face

| Repository | Content |
|---|---|
| [SohamW11/rosfix-assist](https://huggingface.co/SohamW11/rosfix-assist) | The fix ranker: weights, inference code excerpt, model card with held-out results |
| [datasets/SohamW11/rosfix-knowledge-packs](https://huggingface.co/datasets/SohamW11/rosfix-knowledge-packs) | The knowledge packs (Humble, Jazzy, Kilted) and rosdep key lists, with the scripts that rebuild them |

Both are updated by the release workflow on every tagged release (when the repository has an
`HF_TOKEN` secret), or by hand with `HF_USER=SohamW11 scripts/publish_hf.sh`.

## Project layout

- `crates/ros2-safety-lint` — the `rosfix` library and CLI; `packs/` holds the embedded
  knowledge packs and rosdep keys.
- `crates/ros2-safety-mutate` — `rosfix-bench` (mutation benchmark), `rosfix-train` (fix
  ranker), `rosfix-pack` (knowledge-pack builder), `rosfix-history` (replays real fix commits).
- `scripts/` — `build_packs.sh`, `build_rosdep_keys.py`, `publish_hf.sh`.
- `paper/` — the paper and the raw data behind its numbers.
- `CONTRIBUTING.md` — architecture and how to add a rule.

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

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
