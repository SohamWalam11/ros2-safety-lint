# rosfix agents

rosfix has **18 agents** (93 rules): 17, one per problem area, plus `core`, which reports files
it could not parse (`rosfix rules` lists them). Not every agent is described in detail below. This page explains how they are triggered, what each one reads and reports, and how
they are tested.

## How agents are triggered

Agents are dispatched **on demand**: an agent runs only when the workspace contains something
it checks, and, when the workspace is re-analysed after an edit, only when a file it reads
changed. There is no message bus, no async tasks, and agents never call each other. One
analysis works like this:

```
files on disk ──► 1. index ──► 2. resolve ──► 3. agents ──► 4. post-process ──► report / fix
                  (facts)      (launch graph)  (sequential)   (config, suppressions)
```

1. **Index** (`workspace::index`): every relevant file is parsed once into *facts*:
   packages, executables, C++/Python nodes with their parameters, publishers,
   subscriptions, QoS and callbacks, parameter YAML sections, URDF links, joints and
   `<ros2_control>`, SROS2 policies, and parse errors.
2. **Resolve** (`resolve::build`): launch files are evaluated the way ROS 2 would (namespaces,
   remaps, launch arguments, includes, parameter files) into a **launch graph**. Each launched node is linked to the source class that implements it.
3. **Agents** (`agents::run`), dispatched per agent:
   - **inputs:** `inputs()` declares the file kinds the agent's findings depend on, directly
     or through the launch graph (e.g. `control` reads URDF, parameter YAML and manifests;
     `topics` reads everything the graph is built from).
   - **change event:** on a re-analysis, the engine compares file fingerprints with the previous
     run. An agent whose inputs did not change is **not run**: its previous findings are
     reused (`reuse` in `--stats`).
   - **trigger:** otherwise `relevant(cx)`, a cheap check of the facts, decides whether the
     agent has work at all. If not, it is skipped (`skip` in `--stats`).
   - **run:** `run(cx, out)` reads the shared, **read-only** index and graph and appends
     diagnostics.
   - `--agents a,b` restricts dispatch to those agents; `core` always runs.
4. **Post-process:** apply rule settings from `rosfix.toml`, `# rosfix: ignore[...]`
   comments and baselines.

The index and graph work like a read-only blackboard that every agent reads. Because agents
share no mutable state, the order is irrelevant to the results, each agent can be run alone,
and skipping an agent cannot change another agent's findings.

**The fix loop** (`--fix`) and the mutation benchmark re-analyse after in-memory edits. These
re-analyses are change events: a parse cache (`ExtractCache`) re-parses only edited files
(URDF/xacro is never cached because it includes other files), and only agents that read the
edited kinds of files run again. Fixes are kept only if their finding disappears and no new
error appears.

### Cost (Navigation2: 732 files, 50 launch graphs, 1161 nodes; release build, Windows laptop, warm)

| | before | after |
|---|---:|---:|
| full analysis | 2.6–3.8 s | 1.7–2.4 s |
| `--fix` (analysis + verification) | 4.3–5.3 s | 2.3–2.8 s |
| one re-analysis after an edit | ~2.5 s, 732 files parsed | **0.38 s**, 0.8 files parsed, 6.2 of 9 agents run |

The agents themselves take about 25 ms. The costs were parsing, which the cache removes on
re-analysis, and linking nodes to their source. That linking took 0.83 s; it is now memoized
per executable, so it takes 0.17 s. Findings are identical to before on Navigation2 and
ros2/demos. Output is now also deterministic: before, which site of a repeated finding was
reported depended on hash-map order and changed between runs. **The assistant** (`--assist`) runs after the agents, only on findings that
carry a set of valid replacement values (see [`training/README.md`](../training/README.md)).

The design rule for all agents: **when rosfix cannot know, it stays silent.** Facts that could
not be resolved statically (dynamic launch code, plugin-declared parameters, external
packages) are marked as unknown and turn the dependent checks off instead of guessing.

## The agents

Order is the execution order. "Trigger" is the agent's `relevant()` condition.

### 1. `core`: parse errors
- **Trigger:** at least one file failed to parse.
- **Reports:** `R001` (info): file could not be parsed, with the parser's first error line.
- **Why:** nothing fails silently. A broken YAML would otherwise hide every check on it.

### 2. `params`: parameters (`src/agents/params.rs`)
- **Trigger:** any parameter YAML file exists, or any launched node gets parameters.
- **Reads:** parameter files, the parameters each node instance receives in the launch graph,
  and the parameters its source code declares (`declare_parameter`, Python `declare_parameter`,
  knowledge packs for installed nodes).
- **Reports:**
  - `P001`: YAML key matches no launched node (e.g. `driver:` while the node is `/robot1/driver`), so the values are dropped.
  - `P002`: `ros__parameters` missing or misspelled.
  - `P003`: parameter not declared by the node, so it is ignored. Carries the declared names as options for `--assist`.
  - `P004`: wrong type (e.g. integer for a `double`), which throws at startup. Auto-fix `1` → `1.0`.
  - `P005`: a required parameter (declared without a default) is not provided.
- **Silent when:** the node's source is not found or declares parameters dynamically.

### 3. `topics`: topic wiring (`src/agents/topics.rs`)
- **Trigger:** at least one launched node is linked to its source (its endpoints are known).
- **Reads:** fully-resolved topic names of every publisher and subscription per launch graph
  (after namespaces and remaps).
- **Reports:**
  - `T001`: near miss: a subscriber has no publisher, but one exists on the same name in another namespace.
  - `T002`: remap of a name the node never uses, so it has no effect.
  - `T003`: one topic used with two message types.
  - `T004` (info): absolute name inside a namespaced node.
  - `T005` (off by default): subscription without any publisher.

### 4. `executor`: callbacks and deadlocks (`src/agents/executor.rs`)
- **Trigger:** some node's code contains a blocking call (sync service call, future wait,
  spin, sleep).
- **Reads:** callbacks (subscriptions, timers, services; lambdas or member functions, one level
  of calls deep), their callback groups, and the executor the package uses.
- **Reports:**
  - `E001`: a blocking wait or sync call inside a callback whose response arrives through the same callback group or a single-threaded executor, or the node spun from inside its own callback. Either deadlocks. (Spinning a dedicated executor that owns a separate callback group is allowed.)
  - `E002`: a sleep inside a callback, which stalls every other callback on that executor thread.
- **Silent for:** test paths (`test_paths` in `rosfix.toml`), work handed to other threads,
  clients in a dedicated group with a multi-threaded executor.

### 5. `control`: ros2_control (`src/agents/control.rs`)
- **Trigger:** a URDF has a `<ros2_control>` block, or a YAML has a `controller_manager` section.
- **Reads:** URDF joints, `<ros2_control>` joints with command/state interfaces and hardware
  plugins, controller types and their `joints:`.
- **Reports:**
  - `C001`: a controller joint is not in any `<ros2_control>` block.
  - `C002`: the controller needs an interface the joint lacks (e.g. a velocity controller on a position-only joint).
  - `C003`: unknown controller type (typo).
  - `C004`: controller configuration mismatch.
  - `C005`: a `<ros2_control>` joint is missing from the URDF.
  - `C006`: hardware plugin missing.

### 6. `packaging`: build and install (`src/agents/packaging.rs`)
- **Trigger:** the workspace contains at least one package (`package.xml`).
- **Reads:** `package.xml`, `CMakeLists.txt` targets and installs, `setup.py`/`setup.cfg`,
  executables and files referenced by launch files.
- **Reports:**
  - `L001`: a launch file starts an executable that the package does not build or install.
  - `L002`: a dependency is used (launch file, `find_package`, import) but not declared in `package.xml`. Auto-fix adds the `<depend>`.
  - `L003`: Python package without `install_scripts` in `setup.cfg`.
  - `L004`: `launch/` or `config/` used at runtime but never installed.
  - `L005`: a referenced parameter or launch file does not exist.

### 7. `simtime`: clocks (`src/agents/simtime.rs`)
- **Trigger:** some launch graph has more than one node, or starts a simulator.
- **Reports:**
  - `S001`: some nodes use simulated time, others wall-clock time (no simulator in that graph).
  - `S002`: a simulator is launched but a node runs on wall-clock time. With a simulator present, this replaces `S001` so the same node is not reported twice.
  - `S003` (off by default): nodes use sim time but nothing publishes `/clock`.

### 8. `qos`: QoS compatibility (`src/agents/qos.rs`)
- **Trigger:** at least one launch graph exists.
- **Reads:** the QoS profile of every endpoint (code, presets like `SensorDataQoS`, RViz
  displays, knowledge packs) and `qos_overrides` in YAML.
- **Reports:**
  - `Q001`: RELIABLE subscription on a BEST_EFFORT publisher, so no messages arrive.
  - `Q002`: TRANSIENT_LOCAL subscription on a VOLATILE publisher.
  - `Q003`: deadline or liveliness incompatible.
  - `Q004` (info): a late joiner would miss messages.
  - `Q005`: `qos_overrides` for an endpoint that does not accept overrides, so they are silently ignored.

### 9. `nav2`: Nav2 configuration (`src/agents/nav2.rs`)
- **Trigger:** a YAML section of a Nav2 server (controller/planner/costmap/BT navigator, …).
- **Reports:**
  - `N001`: a plugin is listed without its section.
  - `N002`: unknown plugin type (typo).
  - `N003`: a frame is not in the URDF and not a standard frame.
  - `N004`: degenerate footprint.
  - `N005`: inflation radius smaller than the robot.
  - `N006`: parameter-name typo.

### 10. `security`: SROS2 (`src/agents/security.rs`)
- **Trigger:** governance or permissions files, SROS2 policies, or `ROS_SECURITY_*` in a launch environment.
- **Reports:**
  - `X001`: weak governance (unauthenticated participants allowed).
  - `X002`: overly broad grants (`*`, `<default>ALLOW`).
  - `X003`: expired permissions.
  - `X004`: a node uses an enclave that no policy defines.
  - `X005`: a node publishes or subscribes where its policy denies it.
  - `X006` (info): absolute keystore path.

`rosfix --list-rules` prints all rules; `rosfix --explain <ID>` explains one.

## Testing

| Level | Where | What it proves |
|---|---|---|
| Unit | `#[cfg(test)]` in each `src/agents/*.rs` | Each rule fires on a minimal fixture, plus negative cases (e.g. QoS overrides enabled → no `Q001`). |
| Sandbox | [`tests/agents.rs`](../crates/ros2-safety-lint/tests/agents.rs) | A realistic 7-package workspace (`bot`, `arm`, `nav`, `sec`, `tools`, `pyq`, `broken`) with a planted bug for every agent, analysed end to end from disk. |
| CLI | `tests/cli.rs` | Exit codes, text/JSON/SARIF output. |
| Mutation benchmark | `rosfix-bench` (crates/ros2-safety-mutate) | Bugs injected at real sites in real repositories. |

The sandbox tests check, for every agent:

1. **Its trigger fires** on the sandbox (`ran = yes`) and it reports every planted rule.
2. **It stays off** without its trigger: on a bare package only `packaging` runs, and there are no findings.
3. **Selection is isolated:** `--agents <id>` runs only that agent (plus `core`) and reports only its rules.
4. **No false positives** on a clean workspace (talker/listener with parameters, namespace and launch file).
5. Executor messages name inline callbacks correctly ("timer callback (lambda) sleeps").
6. **Incremental equals full** (differential test): 79 edits across every sandbox file
   (renamed identifiers, dropped lines, added lines). After each edit, the incremental
   re-analysis (cache + change event) must report exactly what a fresh full analysis reports.
   An agent that under-declares its `inputs()` fails this test; removing `Urdf` from
   `control`'s inputs was checked to fail it.

Run them, keeping the sandbox on disk to inspect it with the CLI:

```bash
ROSFIX_SANDBOX_DIR=/tmp/rosfix-sandbox cargo test -p rosfix --test agents
rosfix /tmp/rosfix-sandbox --no-ros --stats          # all agents + per-agent timing
rosfix /tmp/rosfix-sandbox --no-ros --agents qos     # one agent
rosfix /tmp/rosfix-sandbox --no-ros --print-graph    # what the resolver understood
```

Sandbox result (release build): 7 packages, 4 launch graphs, 9 nodes, 15 files parsed in about 15 ms.
All 10 agents ran and reported 38 findings (15 errors, 21 warnings, 2 info).

## Adding an agent

1. Create `src/agents/<name>.rs` with a type implementing `Agent` (`id`, `title`,
   `inputs`, `relevant`, `run`). Keep `relevant` cheap and specific: it is the agent's trigger.
   Declare in `inputs` every file kind the agent reads, including through the launch graph
   (`GRAPH_INPUTS`). Over-declaring only costs reruns; under-declaring returns stale findings.
2. Register the rules in `src/rules.rs` (id, agent, severity, summary, explanation).
3. Add it to `all_agents()` in `src/agents/mod.rs`.
4. Add unit tests in the module, a planted bug in the sandbox (`tests/agents.rs`) with its
   rules in `EXPECTED`, and keep `clean_workspace_has_no_findings` green.
