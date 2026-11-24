//! End-to-end tests of the `rosfix` binary on a small, deliberately broken workspace.

use std::path::Path;
use std::process::Command;

const FILES: &[(&str, &str)] = &[
    ("demo/package.xml", "<package format=\"3\">\n  <name>demo</name>\n  <depend>rclcpp</depend>\n</package>\n"),
    (
        "demo/CMakeLists.txt",
        "project(demo)\nfind_package(rclcpp REQUIRED)\nfind_package(std_msgs REQUIRED)\nadd_executable(talker src/talker.cpp)\ninstall(TARGETS talker DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch config DESTINATION share/${PROJECT_NAME})\nament_package()\n",
    ),
    (
        "demo/src/talker.cpp",
        "#include <rclcpp/rclcpp.hpp>\nclass Talker : public rclcpp::Node {\npublic:\n  Talker() : Node(\"talker\") {\n    declare_parameter(\"max_velocity\", 1.0);\n    declare_parameter(\"rate\", 10);\n  }\n};\n",
    ),
    ("demo/config/params.yaml", "talker:\n  ros__parameters:\n    max_velocity: 2\n    rate: 5\n"),
    (
        "demo/launch/demo.launch.xml",
        "<launch>\n  <node pkg=\"demo\" exec=\"talker\">\n    <param from=\"$(find-pkg-share demo)/config/params.yaml\"/>\n  </node>\n</launch>\n",
    ),
];

fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (rel, text) in FILES {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

fn rosfix(root: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_rosfix"))
        .arg(root)
        .args(args)
        .arg("--no-color")
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into(),
        String::from_utf8_lossy(&out.stderr).into(),
    )
}

#[test]
fn reports_and_exit_codes() {
    let ws = workspace();
    let (code, out, _) = rosfix(ws.path(), &[]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("P004"), "{out}");
    assert!(out.contains("L002"), "{out}");
    let (code, _, _) = rosfix(ws.path(), &["--fail-on", "never"]);
    assert_eq!(code, 0);
    let (code, _, err) = rosfix(Path::new("definitely/missing/dir"), &[]);
    assert_eq!(code, 2, "{err}");
}

#[test]
fn machine_readable_output_is_valid() {
    let ws = workspace();
    let (_, out, _) = rosfix(ws.path(), &["--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert!(v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["rule"] == "P004"));
    // --fix must not leak text into machine-readable stdout.
    let (_, out, _) = rosfix(ws.path(), &["--format", "sarif", "--fix"]);
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid SARIF");
    assert_eq!(v["version"], "2.1.0");
    let results = v["runs"][0]["results"].as_array().unwrap();
    assert!(results.iter().all(
        |r| r["locations"][0]["physicalLocation"]["region"]["startLine"]
            .as_u64()
            .unwrap()
            >= 1
    ));
}

#[test]
fn fix_previews_without_writing_then_writes_verified_fixes() {
    let ws = workspace();
    let params = ws.path().join("demo/config/params.yaml");
    let manifest = ws.path().join("demo/package.xml");
    let before = std::fs::read_to_string(&params).unwrap();

    let (_, out, _) = rosfix(ws.path(), &["--fix"]);
    assert!(out.contains("+    max_velocity: 2.0"), "{out}");
    assert_eq!(
        std::fs::read_to_string(&params).unwrap(),
        before,
        "--fix alone must not modify files"
    );

    let (code, _, err) = rosfix(ws.path(), &["--fix", "--write"]);
    assert_eq!(code, 0, "{err}");
    assert!(std::fs::read_to_string(&params)
        .unwrap()
        .contains("max_velocity: 2.0"));
    assert!(std::fs::read_to_string(&manifest)
        .unwrap()
        .contains("<depend>std_msgs</depend>"));

    let (code, out, _) = rosfix(ws.path(), &[]);
    assert_eq!(
        code, 0,
        "after fixing, the workspace should be clean:\n{out}"
    );
}

#[test]
fn suppressions_and_baseline() {
    let ws = workspace();
    let params = ws.path().join("demo/config/params.yaml");
    std::fs::write(
        &params,
        "talker:\n  ros__parameters:\n    max_velocity: 2  # rosfix: ignore[P004]\n    rate: 5\n",
    )
    .unwrap();
    let (_, out, _) = rosfix(ws.path(), &["--format", "json"]);
    assert!(!out.contains("\"P004\""), "{out}");

    let baseline = ws.path().join("baseline.json");
    let (code, _, _) = rosfix(ws.path(), &["--write-baseline", baseline.to_str().unwrap()]);
    assert_eq!(code, 0);
    let (code, _, _) = rosfix(ws.path(), &["--baseline", baseline.to_str().unwrap()]);
    assert_eq!(code, 0);
}
