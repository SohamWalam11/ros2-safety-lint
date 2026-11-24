//! Sandbox tests for `rosfix fix`: a workspace with one planted, mechanically fixable
//! mistake for every rule that has a safe fix, plus traps that must not be touched
//! (a suppressed finding, an ambiguous typo, a one-line and a CRLF package.xml).
//!
//! The tests run the real binary on a copy of the workspace and check, file by file and
//! byte by byte, what `rosfix fix --write` writes; that a dry run writes nothing; that the
//! printed diff is exactly what is written; that the fixed findings are gone and nothing
//! new appears; that a second run finds nothing to fix; and that the old `--fix --write`
//! flags behave the same as the `fix` command.
//!
//! Set `ROSFIX_FIX_SANDBOX_DIR=<dir>` to keep a copy of the unfixed workspace on disk.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

const LAUNCH_PY_HEAD: &str = "from launch import LaunchDescription\nfrom launch_ros.actions import Node\nfrom ament_index_python.packages import get_package_share_directory\nimport os\n";

fn files() -> Vec<(&'static str, String)> {
    let mut f: Vec<(&'static str, String)> = Vec::new();
    let mut add = |p: &'static str, s: &str| f.push((p, s.to_string()));

    // ---- fx: parameters (P001 P002 P003 P004) and packaging (L002 L004)
    add("fx/package.xml", "<?xml version=\"1.0\"?>\n<package format=\"3\">\n  <name>fx</name>\n  <version>0.1.0</version>\n  <depend>rclcpp</depend>\n  <export>\n    <build_type>ament_cmake</build_type>\n  </export>\n</package>\n");
    add("fx/CMakeLists.txt", "cmake_minimum_required(VERSION 3.8)\nproject(fx)\nfind_package(ament_cmake REQUIRED)\nfind_package(rclcpp REQUIRED)\nfind_package(std_msgs REQUIRED)\nadd_executable(driver src/driver.cpp)\ninstall(TARGETS driver DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch DESTINATION share/${PROJECT_NAME})\nament_package()\n");
    add(
        "fx/src/driver.cpp",
        r#"#include <rclcpp/rclcpp.hpp>
#include <std_msgs/msg/string.hpp>
class Driver : public rclcpp::Node {
public:
  Driver() : Node("driver") {
    declare_parameter("wheel_radius", 0.1);
    declare_parameter("wheel_base", 0.3);
    declare_parameter<std::string>("frame_name", "base");
    declare_parameter("max_linear_velocity", 0.5);
    declare_parameter("max_angular_velocity", 1.0);
    pub_ = create_publisher<std_msgs::msg::String>("status", 10);
  }
  rclcpp::Publisher<std_msgs::msg::String>::SharedPtr pub_;
};
int main(int argc, char** argv) { rclcpp::init(argc, argv); rclcpp::spin(std::make_shared<Driver>()); }
"#,
    );
    add(
        "fx/config/driver.yaml",
        "# Driver parameters\ndriver:\n  ros__parameters:\n    wheel_radius: 1\n    wheel_base: 1  # rosfix: ignore[P004]\n    frame_name: 42\n    max_linear_velocty: 0.5\n    max_vel: 2.0\n",
    );
    add("fx/config/extra.yaml", "drivr:\n  ros__parameters:\n    max_angular_velocity: 1.5\n");
    add("fx/config/typo.yaml", "driver:\n  ros__parametres:\n    wheel_radius: 0.2\n");
    add(
        "fx/launch/bringup.launch.py",
        &format!(
            "{LAUNCH_PY_HEAD}def generate_launch_description():\n    share = get_package_share_directory('fx')\n    a = os.path.join(share, 'config', 'driver.yaml')\n    b = os.path.join(share, 'config', 'extra.yaml')\n    c = os.path.join(share, 'config', 'typo.yaml')\n    return LaunchDescription([\n        Node(package='fx', executable='driver', name='driver', parameters=[a, b, c]),\n    ])\n"
        ),
    );

    // ---- slim: one-line package.xml (L002 must insert inside <package>)
    add("slim/package.xml", "<package format=\"3\"><name>slim</name></package>");
    add("slim/CMakeLists.txt", "project(slim)\nfind_package(ament_cmake REQUIRED)\nfind_package(geometry_msgs REQUIRED)\nament_package()\n");

    // ---- crlf: Windows line endings must survive (L002)
    add("crlf/package.xml", "<package format=\"3\">\r\n  <name>crlf</name>\r\n  <depend>rclcpp</depend>\r\n</package>\r\n");
    add("crlf/CMakeLists.txt", "project(crlf)\r\nfind_package(ament_cmake REQUIRED)\r\nfind_package(rclcpp REQUIRED)\r\nfind_package(sensor_msgs REQUIRED)\r\nament_package()\r\n");

    // ---- pyfx: ament_python package without install_scripts (L003)
    add("pyfx/package.xml", "<package format=\"3\"><name>pyfx</name><export><build_type>ament_python</build_type></export></package>");
    add("pyfx/setup.py", "from setuptools import setup\nsetup(name='pyfx', entry_points={'console_scripts': ['n = pyfx.n:main']})\n");
    add("pyfx/setup.cfg", "[metadata]\nname=pyfx\n");

    // ---- nav: Nav2 configuration (N002 N003 N006)
    add("nav/package.xml", "<package format=\"3\"><name>nav</name></package>");
    add("nav/urdf/robot.urdf", "<robot name=\"r\"><link name=\"base_link\"/></robot>");
    add(
        "nav/config/nav2.yaml",
        "controller_server:\n  ros__parameters:\n    controller_frequncy: 20.0\n    controller_plugins: [\"FollowPath\"]\n    FollowPath:\n      plugin: \"nav2_regulated_pure_pursuit_controller::RegulatedPurePursuitControler\"\n      desired_linear_vel: 0.5\nlocal_costmap:\n  local_costmap:\n    ros__parameters:\n      global_frame: odom\n      robot_base_frame: base_lnk\n      robot_radius: 0.3\n      plugins: [\"inflation_layer\"]\n      inflation_layer:\n        plugin: \"nav2_costmap_2d::InflationLayer\"\n        inflation_radius: 0.55\n        cost_scalling_factor: 3.0\n",
    );

    // ---- arm: ros2_control controller type typo (C003)
    add("arm/package.xml", "<package format=\"3\">\n  <name>arm</name>\n  <exec_depend>controller_manager</exec_depend>\n  <exec_depend>launch_ros</exec_depend>\n</package>\n");
    add(
        "arm/urdf/arm.urdf",
        "<robot name=\"arm\"><link name=\"base\"/><link name=\"l1\"/>\n<joint name=\"j1\" type=\"revolute\"><parent link=\"base\"/><child link=\"l1\"/></joint>\n<ros2_control name=\"sys\" type=\"system\"><hardware><plugin>mock_components/GenericSystem</plugin></hardware>\n<joint name=\"j1\"><command_interface name=\"position\"/><state_interface name=\"position\"/></joint>\n</ros2_control></robot>",
    );
    add(
        "arm/config/controllers.yaml",
        "controller_manager:\n  ros__parameters:\n    update_rate: 100\n    jtc:\n      type: joint_trajectory_controler/JointTrajectoryController\njtc:\n  ros__parameters:\n    joints: [j1]\n    command_interfaces: [position]\n    state_interfaces: [position]\n",
    );
    add(
        "arm/launch/control.launch.py",
        &format!(
            "{LAUNCH_PY_HEAD}def generate_launch_description():\n    cfg = os.path.join(get_package_share_directory('arm'), 'config', 'controllers.yaml')\n    return LaunchDescription([\n        Node(package='controller_manager', executable='ros2_control_node', parameters=[cfg]),\n        Node(package='controller_manager', executable='spawner', arguments=['jtc']),\n    ])\n"
        ),
    );

    // ---- tfdemo: frame looked up with a leading slash (F003)
    add("tfdemo/package.xml", "<package format=\"3\">\n  <name>tfdemo</name>\n  <exec_depend>launch_ros</exec_depend>\n</package>\n");
    add(
        "tfdemo/launch/frames.launch.py",
        &format!(
            "{LAUNCH_PY_HEAD}def generate_launch_description():\n    return LaunchDescription([\n        Node(package='tfdemo', executable='scanner', parameters=[{{'odom_frame': '/odom'}}]),\n    ])\n"
        ),
    );
    f
}

// ------------------------------------------------------------------------------ helpers

fn write_workspace(dir: &Path) {
    for (rel, text) in files() {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
}

fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write_workspace(dir.path());
    if let Ok(keep) = std::env::var("ROSFIX_FIX_SANDBOX_DIR") {
        write_workspace(Path::new(&keep));
    }
    dir
}

/// Every file under `root`: relative path (with `/`) → bytes.
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.insert(rel, std::fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// Runs the binary hermetically: no installed ROS, no distribution, no colour.
fn rosfix(root: &Path, args: &[&str]) -> (i32, String, String) {
    // `baseline` writes no report, so it has no --no-color.
    let color = if args.first() == Some(&"baseline") { None } else { Some("--no-color") };
    let out = Command::new(env!("CARGO_BIN_EXE_rosfix"))
        .args(args)
        .arg(root)
        .arg("--no-ros")
        .args(color)
        .env_remove("ROS_DISTRO")
        .env_remove("AMENT_PREFIX_PATH")
        .env_remove("NO_COLOR")
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// (rule, file) of every finding, from the JSON report.
fn findings(root: &Path) -> BTreeSet<(String, String, String)> {
    let (_, out, err) = rosfix(root, &["check", "--format", "json", "--fail-on", "never"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}\n{err}"));
    v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["rule"].as_str().unwrap().to_string(),
                d["location"]["file"].as_str().unwrap().to_string(),
                d["message"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn text(snap: &BTreeMap<String, Vec<u8>>, rel: &str) -> String {
    String::from_utf8(snap[rel].clone()).unwrap()
}

// ------------------------------------------------------------------------------ expected

/// What `rosfix fix --write` must leave in every file it changes. Reviewed by hand:
/// each change is the minimal edit for its finding, and every other byte is unchanged.
fn expected_after() -> BTreeMap<&'static str, String> {
    let mut m = BTreeMap::new();
    let original: BTreeMap<&str, String> = files().into_iter().collect();
    let edit = |rel: &str, pairs: &[(&str, &str)]| {
        let mut t = original[rel].clone();
        for (from, to) in pairs {
            assert_eq!(t.matches(from).count(), 1, "{rel}: `{from}` must occur once");
            t = t.replacen(from, to, 1);
        }
        t
    };
    m.insert(
        "fx/config/driver.yaml",
        edit(
            "fx/config/driver.yaml",
            &[
                ("wheel_radius: 1\n", "wheel_radius: 1.0\n"),
                ("frame_name: 42\n", "frame_name: \"42\"\n"),
                ("max_linear_velocty:", "max_linear_velocity:"),
            ],
        ),
    );
    m.insert("fx/config/extra.yaml", edit("fx/config/extra.yaml", &[("drivr:", "driver:")]));
    m.insert("fx/config/typo.yaml", edit("fx/config/typo.yaml", &[("ros__parametres:", "ros__parameters:")]));
    m.insert(
        "fx/package.xml",
        edit(
            "fx/package.xml",
            &[(
                "  <depend>rclcpp</depend>\n",
                "  <depend>rclcpp</depend>\n  <depend>std_msgs</depend>\n  <exec_depend>launch_ros</exec_depend>\n",
            )],
        ),
    );
    m.insert(
        "fx/CMakeLists.txt",
        edit(
            "fx/CMakeLists.txt",
            &[("ament_package()", "install(DIRECTORY config DESTINATION share/${PROJECT_NAME})\n\nament_package()")],
        ),
    );
    m.insert(
        "slim/package.xml",
        edit("slim/package.xml", &[("</name>", "</name><depend>geometry_msgs</depend>")]),
    );
    m.insert(
        "crlf/package.xml",
        edit(
            "crlf/package.xml",
            &[("  <depend>rclcpp</depend>\r\n", "  <depend>rclcpp</depend>\r\n  <depend>sensor_msgs</depend>\r\n")],
        ),
    );
    m.insert(
        "pyfx/setup.cfg",
        edit(
            "pyfx/setup.cfg",
            &[("name=pyfx\n", "name=pyfx\n[develop]\nscript_dir=$base/lib/pyfx\n[install]\ninstall_scripts=$base/lib/pyfx\n")],
        ),
    );
    m.insert(
        "nav/config/nav2.yaml",
        edit(
            "nav/config/nav2.yaml",
            &[
                ("controller_frequncy:", "controller_frequency:"),
                ("RegulatedPurePursuitControler\"", "RegulatedPurePursuitController\""),
                ("robot_base_frame: base_lnk", "robot_base_frame: base_link"),
                ("cost_scalling_factor:", "cost_scaling_factor:"),
            ],
        ),
    );
    m.insert(
        "arm/config/controllers.yaml",
        edit("arm/config/controllers.yaml", &[("joint_trajectory_controler/", "joint_trajectory_controller/")]),
    );
    m.insert(
        "tfdemo/launch/frames.launch.py",
        edit("tfdemo/launch/frames.launch.py", &[("'odom_frame': '/odom'", "'odom_frame': 'odom'")]),
    );
    m
}

/// Rules whose planted finding has a safe fix in this workspace.
const FIXED_RULES: &[&str] = &[
    "P001", "P002", "P003", "P004", "L002", "L003", "L004", "N002", "N003", "N006", "C003", "F003",
];

// ------------------------------------------------------------------------------ tests

#[test]
fn every_safe_fix_rule_is_planted() {
    let ws = workspace();
    let (_, out, _) = rosfix(ws.path(), &["check", "--format", "json", "--fail-on", "never"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let with_safe_fix: BTreeSet<&str> = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["fix"]["safety"] == "safe")
        .map(|d| d["rule"].as_str().unwrap())
        .collect();
    let want: BTreeSet<&str> = FIXED_RULES.iter().copied().collect();
    assert_eq!(with_safe_fix, want);
}

#[test]
fn dry_run_writes_nothing_and_prints_the_diff() {
    let ws = workspace();
    let before = snapshot(ws.path());
    let (code, out, err) = rosfix(ws.path(), &["fix"]);
    assert_eq!(snapshot(ws.path()), before, "a dry run must not change any file");
    assert_eq!(code, 1, "findings remain: {err}");
    for rel in expected_after().keys() {
        assert!(out.contains(&format!("--- a/{rel}\n+++ b/{rel}\n")), "diff misses {rel}:\n{out}");
    }
    assert!(err.contains("run `rosfix fix --write` to apply"), "{err}");
    assert!(!err.contains("skipped fix"), "{err}");
}

#[test]
fn write_applies_exactly_the_reviewed_edits() {
    let ws = workspace();
    let before = snapshot(ws.path());
    let (_, _, err) = rosfix(ws.path(), &["fix", "--write"]);
    assert!(!err.contains("skipped fix"), "{err}");
    let after = snapshot(ws.path());
    let expected = expected_after();

    // The same set of files, and no temporary files left behind.
    assert_eq!(after.keys().collect::<Vec<_>>(), before.keys().collect::<Vec<_>>());
    for (rel, bytes) in &after {
        match expected.get(rel.as_str()) {
            Some(want) => assert_eq!(&text(&after, rel), want, "{rel}"),
            None => assert_eq!(bytes, &before[rel], "{rel} must not change"),
        }
    }
    // The traps: suppressed and ambiguous findings stay as written.
    let driver = text(&after, "fx/config/driver.yaml");
    assert!(driver.contains("wheel_base: 1  # rosfix: ignore[P004]\n"));
    assert!(driver.contains("max_vel: 2.0\n"));
    // CRLF survived: no bare LF in the CRLF manifest.
    let crlf = text(&after, "crlf/package.xml");
    assert_eq!(crlf.matches('\n').count(), crlf.matches("\r\n").count());
    // Every package.xml is still a well-formed document with one root.
    for (rel, _) in after.iter().filter(|(r, _)| r.ends_with("package.xml")) {
        roxmltree::Document::parse(&text(&after, rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    }
}

#[test]
fn printed_diff_is_what_gets_written() {
    let ws = workspace();
    let before = snapshot(ws.path());
    let (_, shown, _) = rosfix(ws.path(), &["fix"]);
    rosfix(ws.path(), &["fix", "--write"]);
    let after = snapshot(ws.path());
    let mut actual = String::new();
    for (rel, new) in &after {
        let old = &before[rel];
        if old != new {
            let (o, n) = (String::from_utf8_lossy(old), String::from_utf8_lossy(new));
            let d = similar::TextDiff::from_lines(o.as_ref(), n.as_ref());
            actual.push_str(
                &d.unified_diff()
                    .context_radius(3)
                    .header(&format!("a/{rel}"), &format!("b/{rel}"))
                    .to_string(),
            );
        }
    }
    // The dry run's stdout is the text report followed by the diff.
    let start = shown.find("--- a/").expect("diff in output");
    assert_eq!(&shown[start..], actual);
}

#[test]
fn fixed_findings_disappear_and_nothing_new_appears() {
    let ws = workspace();
    let before = findings(ws.path());
    rosfix(ws.path(), &["fix", "--write"]);
    let after = findings(ws.path());
    let new: Vec<_> = after.difference(&before).collect();
    assert!(new.is_empty(), "new findings after fixing: {new:#?}");
    for r in FIXED_RULES {
        // `max_vel` is the ambiguous trap: it has no safe fix and must stay reported.
        let left: Vec<_> = after
            .iter()
            .filter(|(rule, _, m)| rule == r && !m.contains("`max_vel`"))
            .collect();
        assert!(left.is_empty(), "{r} still reported: {left:#?}");
    }
    // The traps are still reported: the suppressed one is hidden, the ambiguous one is not.
    assert!(after.iter().any(|(r, _, m)| r == "P003" && m.contains("max_vel")), "{after:#?}");
}

#[test]
fn second_run_has_nothing_to_fix() {
    let ws = workspace();
    rosfix(ws.path(), &["fix", "--write"]);
    let once = snapshot(ws.path());
    let (_, _, err) = rosfix(ws.path(), &["fix", "--write"]);
    assert!(err.contains("no automatic fixes available"), "{err}");
    assert_eq!(snapshot(ws.path()), once);
}

#[test]
fn old_flags_and_fix_command_write_the_same_files() {
    let a = workspace();
    let b = workspace();
    let (ca, _, _) = rosfix(a.path(), &["fix", "--write"]);
    let (cb, _, _) = rosfix(b.path(), &["--fix", "--write"]);
    assert_eq!(snapshot(a.path()), snapshot(b.path()));
    assert_eq!(ca, cb);
}

#[test]
fn machine_readable_report_keeps_stdout_clean() {
    let ws = workspace();
    let (_, out, err) = rosfix(ws.path(), &["fix", "--format", "json"]);
    serde_json::from_str::<serde_json::Value>(&out).expect("stdout is only the JSON report");
    assert!(err.contains("--- a/fx/config/driver.yaml"), "the diff goes to stderr");
    let (_, out, _) = rosfix(ws.path(), &["fix", "--format", "sarif"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["version"], "2.1.0");
}

#[test]
fn exit_code_counts_only_what_remains_after_writing() {
    // A workspace whose only findings are fixable: after --write nothing is left.
    let dir = tempfile::tempdir().unwrap();
    for (rel, text) in files().into_iter().filter(|(r, _)| r.starts_with("slim/") || r.starts_with("pyfx/")) {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let (dry, _, _) = rosfix(dir.path(), &["fix", "--fail-on", "warning"]);
    assert_eq!(dry, 1);
    let (wrote, _, err) = rosfix(dir.path(), &["fix", "--write", "--fail-on", "warning"]);
    assert_eq!(wrote, 0, "{err}");
    let (again, _, _) = rosfix(dir.path(), &["check", "--fail-on", "info"]);
    assert_eq!(again, 0);
}

#[test]
fn baseline_hidden_findings_are_not_fixed() {
    let ws = workspace();
    let base = ws.path().join("rosfix-baseline.json");
    let (code, _, err) = rosfix(ws.path(), &["baseline", "-o", base.to_str().unwrap()]);
    assert_eq!(code, 0, "{err}");
    std::fs::remove_file(&base).ok();
    // Record the baseline outside the workspace so it is not part of the snapshot.
    let out = tempfile::tempdir().unwrap();
    let base = out.path().join("b.json");
    rosfix(ws.path(), &["baseline", "-o", base.to_str().unwrap()]);
    let before = snapshot(ws.path());
    let (_, _, err) = rosfix(ws.path(), &["fix", "--write", "--baseline", base.to_str().unwrap()]);
    assert!(err.contains("no automatic fixes available"), "{err}");
    assert_eq!(snapshot(ws.path()), before);
}

#[test]
fn assisted_fixes_need_accept_assist() {
    let ws = workspace();
    let before = text(&snapshot(ws.path()), "fx/config/driver.yaml");
    assert!(before.contains("max_vel: 2.0"));
    // --assist without --accept-assist writes only safe fixes.
    rosfix(ws.path(), &["fix", "--assist", "--write"]);
    let safe_only = text(&snapshot(ws.path()), "fx/config/driver.yaml");
    assert!(safe_only.contains("max_vel: 2.0"), "{safe_only}");
    assert_eq!(safe_only, expected_after()["fx/config/driver.yaml"]);
}
