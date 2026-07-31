# rosfix

[![Cargo Test](https://github.com/SohamWalam11/ros2-safety-lint/actions/workflows/ci.yml/badge.svg)](https://github.com/SohamWalam11/ros2-safety-lint/actions)
[![Crates.io](https://img.shields.io/crates/v/rosfix.svg)](https://crates.io/crates/rosfix)
[![License: MIT/Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)

A fast, multi-language static analysis and auto-remediation tool for ROS 2 codebases.

ROS 2 configurations are usually scattered across launch files, YAML parameters, Python nodes, and C++ code. Often, architects set strict QoS and security rules in the configuration files, but developers accidentally override them by hardcoding bad settings directly into their C++ or Python code. We call this "Shadow Code."

`rosfix` scans your entire workspace across C++, Python, XML, YAML, and URDF files to catch these issues. When run with `--fix`, it acts as a concurrent 7-Agent system (built on `tokio`) that automatically finds and fixes these errors in place.

---

## What It Catches

- **Shadow Code Overrides**: Finds hardcoded `.best_effort()` or bad QoS calls in C++ and Python that bypass global YAML/XML configs.
- **Executor Deadlocks**: Finds blocking `.get()`, `wait_for()`, `sleep_for()`, and `spin_until_future_complete()` calls inside single-threaded subscriber callbacks.
- **Real-Time Heap Allocations**: Flags dynamic memory allocations (`malloc`, `new`, `make_shared`, `push_back`) inside fast control loops.
- **Managed Lifecycle Violations**: Checks that safety-critical actuator nodes use proper lifecycle state transitions (`on_configure`, `on_activate`).
- **Physical Kinematic & Costmap Hazards**: Flags URDF joints missing velocity/effort limits and Nav2 costmaps configured with zero robot radius.
- **SROS2 & Manifest Risks**: Detects wildcard permissions (`<subject>*</subject>`), unencrypted RTPS modes, missing crash policies, and legacy ROS package formats.

---

## Installation

### From Crates.io

```bash
cargo install rosfix
```

### From Source

```bash
git clone https://github.com/SohamWalam11/ros2-safety-lint.git
cd ros2-safety-lint
cargo build --release
```

---

## Usage

### Scanning a Workspace

```bash
# Scan the current directory with colorized terminal output
rosfix

# Scan a specific directory
rosfix --path src/
```

### Automatic In-Place Fixes (`--fix` & Multi-Agent System)

Instead of just reporting errors, running `rosfix` with the `--fix` flag turns it into an automated repair tool using a Multi-Agent System (MAS). 

When you run `--fix`:
1. **The Blackboard**: Detected errors are posted to a shared, thread-safe "Blackboard."
2. **Expert Agents**: 7 specialized agents (like a *KinematicsAgent* or a *BuildSystemAgent*) run concurrently. They constantly check the blackboard and grab tasks they know how to fix.
3. **The 4-Stage Fix Loop**: When an agent takes a task, it goes through four steps: generate a patch, verify it builds with Colcon, run automated tests, and finally apply the fix to the disk.

```bash
# Preview changes without modifying files on disk
rosfix --fix --dry-run

# Automatically scan and apply fixes in-place
rosfix --fix
```

While these agents run, you'll see live progress spinners, and they will safely rewrite files on disk to fix:
- **Launch XMLs:** Adding missing `respawn="true"` crash policies.
- **SROS2 Governance:** Upgrading weak security to `ENCRYPT`.
- **Package Manifests:** Updating legacy `package.xml` formats and adding open-source licenses.
- **Parameter YAMLs:** Fixing unisolated ROS domains and zero-radius footprints.

---

## Output Formats

```bash
# Standard interactive tree format (Line & Col numbers grouped by file)
rosfix
rosfix --format text

# JSON output for custom tools
rosfix --format json

# SARIF v2.1.0 output for CI/CD integration
rosfix --format sarif > results.sarif
```

---

## Configuration (`rosfix.toml`)

Drop a `rosfix.toml` file in your workspace root to configure path ignores and custom rules:

```toml
ignore_paths = ["build/*", "install/*", "vendor/*", "target/*"]

[rules]
require_encryption = true
max_qos_history = 10
allow_best_effort = false
```

---

## GitHub Actions Integration

Add `.github/workflows/rosfix.yml` to run automated safety checks on pull requests:

```yaml
name: ROS 2 Safety Audit

on: [push, pull_request]

jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable

      - name: Install rosfix
        run: cargo install rosfix

      - name: Run Audit
        run: rosfix --format sarif > rosfix.sarif

      - name: Upload SARIF Results
        uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: rosfix.sarif
```

---

## Benchmark Metrics

Tested across 12 open-source ROS 2 repositories using Rayon multi-threading:

| Repository | Focus Domain | Files Scanned | Total Time (ms) | Avg Time / File |
| :--- | :--- | :--- | :--- | :--- |
| **PX4-Autopilot** | Flight Control Stack | 26,404 | 474,304 | 17.96 ms |
| **Autoware** | Autonomous Vehicle Stack | 8,074 | 161,939 | 20.06 ms |
| **MoveIt2** | Motion Planning | 1,567 | 34,387 | 21.94 ms |
| **Navigation2** | AMR Navigation | 1,333 | 29,368 | 22.03 ms |
| **rmf_ros2** | Fleet Management | 695 | 13,445 | 19.35 ms |
| **ros2_control** | Hardware Controllers | 314 | 4,582 | 14.59 ms |
| **Total Corpus** | **Full Benchmark Suite** | **38,917** | **727,709** | **18.68 ms** |

---

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
