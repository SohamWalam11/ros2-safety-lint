# rosfix experiments

Reproducible experiments behind the claims in the README and the paper. All numbers come from
release builds on a Windows 11 laptop (16 GB RAM, under memory pressure from other programs),
single-threaded, on shallow clones of public repositories. Treat absolute times as indicative
and relative numbers (same process, same run) as the result.

Corpus (`git clone --depth 1`): ros2/demos, ros-navigation/navigation2, moveit/moveit2,
ros-controls/ros2_control_demos, ros-controls/ros2_controllers, ROBOTIS-GIT/turtlebot3,
SteveMacenski/slam_toolbox, cra-ros-pkg/robot_localization.

## 1. Precision on clean repositories

Every new check was run on all eight repositories before it was kept; a check stays only if
every finding there is a real problem. Checks were tuned to stay silent when a fact cannot
be known statically (dynamic launch code, unresolved substitutions, unknown validators).

| Check (rules) | Findings on the 8 repositories | Status |
|---|---:|---|
| tf (F001–F004) | 0 | — |
| lifecycle (M001–M002) | 0 | — |
| controllers spawned (C007–C008) | 0 | — |
| launch arguments (A001) | 0 | — |
| launch arguments unused (A002, info) | 14 | all real (arguments declared and never used, copied across EKF examples) |
| parameter constraints (P006, C009) | 0 | — |
| components (L001 components, L006) | 0 | — |
| interfaces (L007–L008) | 0 (6 interface packages) | — |
| middleware environment (D001–D004) | 0 (no repository sets these from launch files) | — |
| controller update rates (C010) | 1 | real: ros2_control_demos example_1, `joint_state_broadcaster` 50 Hz under a 10 Hz controller manager |
| missing files (L005, after the `PathSubstitution` fix) | +2 | real: ros2_control_demos example_15 default publisher configs do not exist |

Independent manual review by a second session of the full rule set on 12 repositories:
96 findings, all true positives (E002×3, L002×80, P003×3, S002×10).

## 2. Planted bugs in real code

Each check was validated by planting its bug in a copy of a real repository file and
requiring the finding at the right location:

| Rule | Repository / file | Planted change |
|---|---|---|
| M001, M002 | navigation2 `nav2_map_server/launch/map_saver_server.launch.py` | `node_names: ['map_saver_srv']` |
| C007 | ros2_control_demos `example_2/bringup/launch/diffbot.launch.py` | spawner `diffbot_base_controler` |
| C009 | ros2_control_demos + ros2_controllers | `wheel_separation: -0.10` (spec: `gt<>: [0.0]`) |
| C010 | ros2_control_demos `example_1` | controller `update_rate` above the manager's |
| A001 | slam_toolbox `launch/online_async_launch.py` | `LaunchConfiguration('use_sim_tme')` |
| L001, L006 | demos `composition` | plugin `composition::Talkr`; removed `RCLCPP_COMPONENTS_REGISTER_NODE` |
| L007, L008 | navigation2 `nav2_msgs` | unlisted `.msg`, listed missing `.msg`, removed group membership |
| D001, D002 | demos `talker_listener_launch.py` | `ROS_DOMAIN_ID=233`, `RMW_IMPLEMENTATION=rmw_fastrtps_cp` |
| P007 | turtlebot3 `param/humble/burger.yaml` analysed as Jazzy | (unchanged real file) → `progress_checker_plugin` renamed |

The mutation benchmark (`rosfix-bench`) detects 161/161 single-fault mutations at real sites
on 12 repositories (re-measured 24 Nov 2026 at the revisions in
`paper/data/mutation_bench_revisions.txt`; an earlier run on older revisions found 165/165).

## 3. Real fix commits (`rosfix-history`)

> Recall on real bugs is measured by the labelled study in `research/s1_mined/`
> (results: `research/data/s1_results.md`; 186 repositories, 4,489 candidate commits, blind
> labels with a second rater). Cite that study for recall. The run below is a small,
> reproducible example of the `rosfix-history` tool on 6 repositories.

```bash
git clone --filter=blob:none --no-checkout https://github.com/<repo>.git   # history, no blobs
rosfix-history <repos>... --max-commits 1500 --out report.md --jsonl cases.jsonl
```

Candidates: non-merge commits whose message looks like a fix and that only modify ROS
configuration files (≤ 5). The parent is analysed, then the same tree with the commit's
versions of those files in memory. *Detected* = a finding in a package the commit modified
disappears (packages, not files: a fix often edits package.xml while the finding is reported
where the dependency is used).

| repository | candidates | detected |
|---|---:|---:|
| demos | 24 | 2 |
| navigation2 | 51 | 2 |
| robot_localization | 35 | 0 |
| ros2_control_demos | 29 | 1 |
| slam_toolbox | 13 | 1 |
| turtlebot3 | 72 | 3 |
| **all** | **224** | **9** |

All 9 detections match what the commit says it fixes:

| commit | fix (commit subject) | rosfix finding before the fix |
|---|---|---|
| demos `dd282ffe55` | add missing launch_ros exec dependency (#277) | L002 `launch_ros` imported by a launch file |
| demos `abcfe53757` | Install the launch file for lifecycle_py (#586) | L004 `launch/` never installed |
| turtlebot3 `7a038d067e` | Add missing exec_depend for nav2_bringup | L002 `nav2_bringup` used by a launch file |
| turtlebot3 `81d01664f6` | Add depends | L002 `robot_state_publisher`, `rviz2` |
| turtlebot3 `01afa42412` | Fix turtlebot3_bringup dependencies | L002 `turtlebot3_node` |
| slam_toolbox `aa5cb321fa` | fix rosdep keys | L002 `rclcpp` |
| navigation2 `1856b67b0a` | Fix missing dependency (#5460) | L002 `nav2_simple_commander` |
| navigation2 `1dc23193c1` | Fix typo (#3569) | P003 `global_flame` |
| ros2_control_demos `f5e163d4e0` | correct diffdrive demo (#94) | C008 controller configured, never spawned |

One fix introduced a finding (demos `d7034344c9`: `find_package(builtin_interfaces)` added
without a package.xml dependency, L002), which is accurate.

**Interpretation.** 9 of 224 (4%) is a lower bound on recall: the candidate filter admits
every fix-worded configuration commit, including formatting, version bumps, ROS 1-era
history and changes outside rosfix's scope. Over the first run of this benchmark, two
methodology and coverage problems were found and fixed: scoping detections to modified
*files* hid dependency fixes (3 → 6 detections), and imports in Python launch files were not
counted as dependencies (now L002, info severity, since `launch_ros` is usually installed
anyway). The per-commit JSONL is the input for labelling which misses are in scope.

## 4. Fix assistant: learned ranker vs lexical ranker (`rosfix-train`)

Held-out packages (never seen in training), same abstention rule as at runtime:

| | correct | abstained | wrong |
|---|---:|---:|---:|
| answer offered — lexical | 2213 | 2167 | 19 |
| answer offered — learned | **3816** | 555 | 28 |
| answer absent — lexical | — | 2829 | 123 |
| answer absent — learned | — | **2848** | **104** |

Total wrong edits: lexical 142, learned 132. Model: 321 parameters (1.3 KB), pure Rust,
compiled into the binary (replaces a 469 MB quantised LLM).

## 5. Incremental re-analysis

`rosfix-bench --ablation`: every mutation is re-analysed fully, with the parse cache only,
and with cache + agent reuse + launch-graph reuse; findings must be identical.

| workspace | re-analyses | full (ms) | parse cache only | cache + reuse | result mismatches |
|---|---:|---:|---:|---:|---:|
| ros2_control_demos | 17 | 192 | 101 | 108 | 0 |
| demos | 16 | 197 | 59 | 61 | 0 |
| turtlebot3 | 21 | 69 | 28 | 30 | 0 |

On small workspaces resolve and agents take 0–30 ms, so reuse adds nothing beyond the cache.
On navigation2 (732 files, 50 launch graphs, 1161 nodes; in-process medians of 7):

| edit | resolve without graph reuse | with graph reuse |
|---|---:|---:|
| `nav2_params.yaml` (read by most graphs) | 531 ms | 242 ms |
| a launch file read by one graph | 422 ms | 88 ms |

Per re-analysis after one edit on navigation2: 0.38 s (full analysis 1.7–2.4 s); `--watch`
reports the change in ~1.2 s in a debug build. Correctness is checked by a differential test
(`tests/agents.rs::incremental_reanalysis_equals_full_analysis`, 79 edits) that fails when a
dependency is not tracked (verified by removing URDF from the control agent's inputs, and by
not recording parameter-file dependencies of launch graphs).

Not cached on purpose: URDF/xacro (includes other files) — ros2_control_demos still re-parses
~46 files per edit because of its xacro descriptions.

## Reproduce

```bash
cargo build --release -p rosfix -p ros2-safety-mutate
rosfix-bench <workspaces>... --max-per-op 8 --ablation --out ablation.md
rosfix-history <repos-with-history>... --out history.md --jsonl history.jsonl
cargo run --release -p ros2-safety-mutate --bin rosfix-train
cargo test -p rosfix            # unit, sandbox (incl. differential) and CLI tests
```
