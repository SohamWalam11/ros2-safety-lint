# rosfix

[![Cargo Test](https://github.com/SohamWalam11/ros2-safety-lint/actions/workflows/ci.yml/badge.svg)](https://github.com/SohamWalam11/ros2-safety-lint/actions)
[![Crates.io](https://img.shields.io/crates/v/rosfix.svg)](https://crates.io/crates/rosfix)
[![License: MIT/Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)

> **High-Performance Multi-Language Static Verification and Active Remediation Engine for ROS 2 Security, Reliability, and Physical Safety.**

---

## 📌 Overview

`rosfix` is an open-source, high-speed static analyzer and active remediation engine engineered in Rust for safety-critical ROS 2 robotics codebases (including autonomous vehicles, manipulators, UAVs, and fleet management systems).

Unlike traditional linters that only inspect isolated configuration files ("XML Myopia"), `rosfix` unifies **5 distinct file formats** (`.xml`, `.yaml`, `.py`, `.cpp`, `.urdf`) into a multi-core AST compiler engine. It bridges the gap between high-level architectural declarations and low-level source code execution, eliminating **Shadow Code Vulnerabilities**, **Executor Deadlocks**, and **Physical Hazards** before code hits real hardware.

---

## ✨ Key Features

- **⚡ Multi-Core Rayon Parallelism**: Scans 38,000+ files across multi-gigabyte ROS 2 workspaces in sub-30 seconds (~18.68 ms/file).
- **🛠️ Active Auto-Remediation Engine (`--fix`)**: Automatically repairs detected safety violations in-place atomically.
- **🛡️ Shadow Code & Executor Deadlock Detection**: Tree-Sitter C++ and RustPython AST taint tracking flagging blocking `.get()`, `spin_until_future_complete`, and `sleep` calls inside callbacks.
- **🦾 Physical Kinematic & Nav2 Hazard Verification**: Validates URDF joint `<limit>` velocity/effort numerical bounds and Nav2 costmap footprints (`robot_radius`).
- **⚙️ Workspace Invariant Specification (`rosfix.toml`)**: Custom workspace rules, path ignores, and QoS threshold overrides.
- **🎨 Compiler-Style Diagnostic Options**: Text, JSON, SARIF 2.1.0 (GitHub Actions annotations), and `fancy` color terminal reporting.

---

## 🚀 Installation

### Option 1: Via Cargo (Recommended)

```bash
# Install the CLI binary & cargo subcommand
cargo install rosfix
```

### Option 2: Building From Source

```bash
# Clone repository
git clone https://github.com/SohamWalam11/ros2-safety-lint.git
cd ros2-safety-lint

# Build release executable
cargo build --release

# The binary will be available at target/release/rosfix
```

---

## 💻 Usage Guide

### Basic Directory Scanning

```bash
# Scan a ROS 2 workspace or src directory with rich terminal output
rosfix --path src/ --format fancy

# Or use the native Cargo subcommand
cargo rosfix --path src/ --format fancy
```

### Automated In-Place Remediation (`--fix`)

```bash
# Preview fixes safely without touching disk
rosfix --path src/ --fix --dry-run

# Automatically fix violations in-place
rosfix --path src/ --fix
```

### Output Formats

```bash
# Standard text format
rosfix --path src/ --format text

# Structured JSON format for custom tooling
rosfix --path src/ --format json

# SARIF 2.1.0 format for GitHub Actions PR annotations
rosfix --path src/ --format sarif > results.sarif
```

---

## ⚙️ Workspace Configuration (`rosfix.toml`)

Create a `rosfix.toml` file in your project root to enforce custom topological invariants:

```toml
ignore_paths = ["vendor/*", "build/*", "target/*", "third_party/*"]

[rules]
require_encryption = true
max_qos_history = 10
allow_best_effort = false
```

---

## 🐙 GitHub Actions CI/CD Integration

Add `.github/workflows/rosfix.yml` to your repository to automatically annotate Pull Requests:

```yaml
name: ROS 2 Safety Audit

on: [push, pull_request]

jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Install rosfix
        run: cargo install rosfix

      - name: Run Safety Audit
        run: rosfix --path src/ --format sarif > rosfix.sarif

      - name: Upload SARIF report
        uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: rosfix.sarif
```

---

## 📊 Empirical Benchmarks Across 12 Repositories (38,917 Files)

| Repository | Focus Domain | Files Scanned | Total Time (ms) | Avg Time / File |
| :--- | :--- | :--- | :--- | :--- |
| **PX4-Autopilot** | Drone / UAV Flight Control | 26,404 | 474,304 | 17.96 ms |
| **Autoware** | Autonomous Driving Stack | 8,074 | 161,939 | 20.06 ms |
| **MoveIt2** | Manipulator Motion Planning | 1,567 | 34,387 | 21.94 ms |
| **Navigation2** | AMR Mobile Navigation | 1,333 | 29,368 | 22.03 ms |
| **rmf_ros2** | Fleet Management Framework | 695 | 13,445 | 19.35 ms |
| **ros2_control** | Real-Time Hardware Controllers | 314 | 4,582 | 14.59 ms |
| **Total Ecosystem** | **Full ROS 2 Ecosystem** | **38,917** | **727,709** | **18.68 ms** |

---

## 📄 License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
