# ros2-safety-lint 🚀🛡️

The ultimate static analysis and vulnerability verification tool for ROS 2. 
`ros2-safety-lint` scans your entire robotics workspace (XML, YAML, Python, C++, URDF) for security flaws, missing encryptions, reliability downgrades, and architectural bypasses.

## Features
- **SROS2 Security Analysis:** Detects unencrypted topics in `permissions.xml` and `governance.xml`.
- **QoS Architecture Audits:** Finds dangerous `BEST_EFFORT` downgrades in YAML configs and hardcoded deep inside C++ nodes (`.cpp`, `.hpp`).
- **Network Safety:** Walks Python ASTs to find hardcoded loopback IPs (`127.0.0.1`).
- **Physical Safety:** Parses URDF and Xacro files for joints missing critical velocity/effort `<limit>` boundaries.
- **Blazing Fast:** Written in highly optimized Rust. Scans massive 10,000+ file autonomous driving repositories in under 30 seconds.

## Installation
```bash
cargo install ros2-safety-lint
```

## Usage
Run it against a single file:
```bash
ros2-safety-lint --path src/my_package/config/params.yaml
```

Run it against your entire workspace (outputs standard Text):
```bash
find src/ -type f -exec ros2-safety-lint --path {} \;
```

## GitHub Actions CI/CD (SARIF Output)
You can automatically run `ros2-safety-lint` on every Pull Request. With `--format sarif`, GitHub will place red squiggly lines directly on the vulnerable code.

Create `.github/workflows/ros2-safety-lint.yml`:
```yaml
name: ROS 2 Safety
on: [push, pull_request]
jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo install ros2-safety-lint
      - run: |
          mkdir -p .results
          # Scan workspace and generate SARIF
          find . -type f \( -name "*.xml" -o -name "*.yaml" -o -name "*.py" -o -name "*.cpp" -o -name "*.urdf" \) -exec ros2-safety-lint --path {} --format sarif \; > .results/ros2-safety-lint.sarif
      - uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: .results/ros2-safety-lint.sarif
```
