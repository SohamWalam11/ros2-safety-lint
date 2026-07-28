# Contributing to rosfix

We welcome contributions from robotics engineers, static analysis researchers, and open-source developers.

## Getting Started

1. **Fork and Clone the Repository**:
   ```bash
   git clone https://github.com/SohamWalam11/ros2-safety-lint.git
   cd ros2-safety-lint
   ```

2. **Run the Test Suite**:
   ```bash
   cargo test
   ```

## Adding New Safety Rules

Rules are located in `crates/ros2-safety-lint/src/`:
- `cpp_parser.rs`: C++ Tree-Sitter AST rules (deadlocks, heap allocations, QoS overrides).
- `python_parser.rs`: RustPython AST rules (blocking calls, QoS degradation).
- `lifecycle_parser.rs`: Managed Lifecycle Node state machine verification.
- `urdf_parser.rs`: Kinematic joint limits and collision geometry.
- `yaml_parser.rs`: Parameter profiles and Nav2 costmap footprints.

When adding a new rule:
1. Implement the detection logic returning `LintViolation`.
2. Add a corresponding `#[test]` verifying both positive and negative cases.
3. Verify that all tests pass: `cargo test`.
