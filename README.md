# rosfix

[![Cargo Test](https://github.com/SohamWalam11/ros2-safety-lint/actions/workflows/ci.yml/badge.svg)](https://github.com/SohamWalam11/ros2-safety-lint/actions)
[![Crates.io](https://img.shields.io/crates/v/rosfix.svg)](https://crates.io/crates/rosfix)
[![License: MIT/Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)

Static analysis and auto-remediation tool for ROS 2 codebases.

ROS 2 configurations are often split across launch files, YAML parameter manifests, Python nodes, and C++ source code. Architects set QoS and security rules in configuration manifests, but developers frequently override them directly inside C++ or Python code (creating "Shadow Code" drift) or introduce callback deadlocks and real-time heap allocations.

`rosfix` scans C++, Python, XML, YAML, and URDF files across your workspace to catch these issues early and fix them automatically (`--fix`).

---

## What It Catches

- **Shadow Code Overrides**: Detects hardcoded `.best_effort()` or unreliable QoS calls in C++ and Python nodes that bypass global YAML/XML configs.
- **Executor Deadlocks**: Identifies blocking `.get()`, `wait_for()`, `sleep_for()`, and `spin_until_future_complete()` calls inside single-threaded subscriber callbacks.
- **Real-Time Heap Allocations**: Flags dynamic memory allocations (`malloc`, `new`, `make_shared`, `push_back`) inside high-frequency control loops.
- **Managed Lifecycle Violations**: Checks that safety-critical actuator nodes use proper lifecycle state machine transitions (`on_configure`, `on_activate`).
- **Physical Kinematic & Costmap Hazards**: Flags URDF joints missing velocity/effort limits and Nav2 costmaps configured with zero robot radius.
- **SROS2 & Manifest Risks**: Detects wildcard permissions (`<subject>*</subject>`), unencrypted RTPS protection modes, missing crash respawn policies, and legacy ROS package formats.

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
# Scan a directory with colorized terminal output
rosfix --path src/ --format fancy

# Or via Cargo subcommand
cargo rosfix --path src/ --format fancy
```

### Automatic In-Place Fixes (`--fix`)

`rosfix` can automatically repair detected manifest and configuration issues in-place:

```bash
# Preview changes without modifying files on disk
rosfix --path src/ --fix --dry-run

# Apply fixes in-place
rosfix --path src/ --fix
```

Auto-remediation currently handles:
- Injecting `respawn="true"` into launch `<node>` tags missing crash policies.
- Upgrading `governance.xml` protection modes from `NONE`/`SIGN` to `ENCRYPT`.
- Converting `package.xml` formats (`format="1"`/`"2"` $\to$ `format="3"`) and adding open-source license tags.
- Adjusting unisolated `ROS_DOMAIN_ID: 0` and zero Nav2 footprint radii.

---

## Output Formats

```bash
# Standard line format
rosfix --path src/ --format text

# JSON output for custom tools
rosfix --path src/ --format json

# SARIF v2.1.0 output for CI/CD integration
rosfix --path src/ --format sarif > results.sarif
```

---

## Configuration (`rosfix.toml`)

Place a `rosfix.toml` file in your workspace root to configure path ignores and custom rules:

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
        run: rosfix --path src/ --format sarif > rosfix.sarif

      - name: Upload SARIF Results
        uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: rosfix.sarif
```

---

## Benchmark Metrics

Evaluated across 12 open-source ROS 2 repositories using Rayon data-parallelism:

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
