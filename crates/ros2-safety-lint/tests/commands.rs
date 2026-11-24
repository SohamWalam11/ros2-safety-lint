//! End-to-end tests of the subcommand interface: routing between the old flags and the
//! commands, and the commands that do not analyse a workspace for findings (graph, rules,
//! explain, baseline, init, doctor, migrate, docs).

use std::path::Path;
use std::process::Command;

fn run(args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_rosfix"));
    c.args(args)
        .env_remove("ROS_DISTRO")
        .env_remove("AMENT_PREFIX_PATH")
        .env_remove("RMW_IMPLEMENTATION")
        .env_remove("ROS_DOMAIN_ID")
        .env_remove("ROS_LOCALHOST_ONLY")
        .env_remove("ROS_AUTOMATIC_DISCOVERY_RANGE")
        .env_remove("CYCLONEDDS_URI")
        .env_remove("FASTRTPS_DEFAULT_PROFILES_FILE")
        .env_remove("FASTDDS_DEFAULT_PROFILES_FILE");
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let files: &[(&str, &str)] = &[
        ("demo/package.xml", "<package format=\"3\">\n  <name>demo</name>\n  <depend>rclcpp</depend>\n  <depend>std_msgs</depend>\n</package>\n"),
        ("demo/CMakeLists.txt", "project(demo)\nfind_package(rclcpp REQUIRED)\nfind_package(std_msgs REQUIRED)\nadd_executable(talker src/talker.cpp)\nadd_executable(listener src/listener.cpp)\ninstall(TARGETS talker listener DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch config DESTINATION share/${PROJECT_NAME})\nament_package()\n"),
        ("demo/src/talker.cpp", "#include <rclcpp/rclcpp.hpp>\n#include <std_msgs/msg/string.hpp>\nclass Talker : public rclcpp::Node {\npublic:\n  Talker() : Node(\"talker\") {\n    declare_parameter(\"rate\", 1.0);\n    pub_ = create_publisher<std_msgs::msg::String>(\"chatter\", 10);\n    timer_ = create_wall_timer(std::chrono::milliseconds(250), [this]() {});\n  }\n  rclcpp::Publisher<std_msgs::msg::String>::SharedPtr pub_;\n  rclcpp::TimerBase::SharedPtr timer_;\n};\n"),
        ("demo/src/listener.cpp", "#include <rclcpp/rclcpp.hpp>\n#include <std_msgs/msg/string.hpp>\nclass Listener : public rclcpp::Node {\npublic:\n  Listener() : Node(\"listener\") {\n    sub_ = create_subscription<std_msgs::msg::String>(\"chatter\", 10, [](std_msgs::msg::String::SharedPtr) {});\n  }\n  rclcpp::Subscription<std_msgs::msg::String>::SharedPtr sub_;\n};\n"),
        ("demo/config/talker.yaml", "talker:\n  ros__parameters:\n    rate: 2\n"),
        ("demo/launch/demo.launch.xml", "<launch>\n  <node pkg=\"demo\" exec=\"talker\">\n    <param from=\"$(find-pkg-share demo)/config/talker.yaml\"/>\n  </node>\n  <node pkg=\"demo\" exec=\"listener\" name=\"say &quot;hi&quot;\"/>\n</launch>\n"),
    ];
    for (rel, text) in files {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

fn p(d: &Path) -> &str {
    d.to_str().unwrap()
}

#[test]
fn bare_path_and_check_command_give_the_same_report() {
    let ws = workspace();
    let a = run(
        &[p(ws.path()), "--no-ros", "--no-color", "--format", "json"],
        &[],
    );
    let b = run(
        &[
            "check",
            p(ws.path()),
            "--no-ros",
            "--no-color",
            "--format",
            "json",
        ],
        &[],
    );
    assert_eq!(a.0, b.0);
    let strip = |s: &str| {
        let mut v: serde_json::Value = serde_json::from_str(s).unwrap();
        v["timings_ms"] = serde_json::Value::Null;
        for a in v["agents"].as_array_mut().unwrap() {
            a["micros"] = serde_json::Value::Null;
        }
        v
    };
    assert_eq!(strip(&a.1), strip(&b.1));
    assert!(a.1.contains("\"P004\""), "{}", a.1);
}

#[test]
fn rdjson_carries_verified_fixes_as_suggestions() {
    let ws = workspace();
    let (_, out, _) = run(
        &[
            "check",
            p(ws.path()),
            "--no-ros",
            "--format",
            "rdjson",
            "--fail-on",
            "never",
        ],
        &[],
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["source"]["name"], "rosfix");
    let d = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"]["value"] == "P004")
        .unwrap_or_else(|| panic!("{out}"));
    assert_eq!(d["location"]["path"], "demo/config/talker.yaml");
    assert_eq!(d["location"]["range"]["start"]["line"], 3);
    let s = &d["suggestions"][0];
    assert_eq!(s["text"], "2.0", "{out}");
    assert_eq!(
        (
            s["range"]["start"]["column"].as_u64(),
            s["range"]["end"]["column"].as_u64()
        ),
        (Some(11), Some(12))
    );
}

#[test]
fn html_report_has_findings_agents_and_graph() {
    let ws = workspace();
    let out = ws.path().join("report.html");
    let (code, _, err) = run(&["report", p(ws.path()), "--no-ros", "-o", p(&out)], &[]);
    assert_eq!(code, 0, "{err}");
    let html = std::fs::read_to_string(&out).unwrap();
    assert!(html.starts_with("<!doctype html>"));
    assert!(
        html.contains("<h3>demo/config/talker.yaml</h3>") && html.contains(">P004<"),
        "{html}"
    );
    assert!(html.contains("<b>params</b>"), "agents table");
    assert!(
        html.contains("<pre class=\"mermaid\">flowchart LR"),
        "launch graph"
    );
    // Node names are escaped, not injected into the page.
    assert!(
        html.contains("say #quot;hi#quot;") && !html.contains("say \"hi\""),
        "{html}"
    );
}

#[test]
fn help_lists_commands_even_after_a_path() {
    for args in [&["--help"][..], &[".", "--help"][..]] {
        let (code, out, _) = run(args, &[]);
        assert_eq!(code, 0);
        for c in ["check", "fix", "graph", "init", "doctor", "migrate", "docs"] {
            assert!(out.contains(&format!("  {c} ")), "{c} missing in:\n{out}");
        }
        assert!(out.contains("rosfix [PATH] [OPTIONS]"));
    }
}

#[test]
fn rules_and_explain_match_the_old_flags() {
    assert_eq!(run(&["rules"], &[]).1, run(&["--list-rules"], &[]).1);
    let (code, out, _) = run(&["explain", "P003"], &[]);
    assert_eq!(code, 0);
    assert_eq!(out, run(&["--explain", "P003"], &[]).1);
    let (code, _, err) = run(&["explain", "Z999"], &[]);
    assert_eq!(code, 2);
    assert!(err.contains("unknown rule `Z999`"), "{err}");
}

#[test]
fn graph_formats() {
    let ws = workspace();
    let base = [p(ws.path()), "--no-ros"];
    let (code, text, _) = run(&[&["graph"][..], &base].concat(), &[]);
    assert_eq!(code, 0);
    assert!(
        text.contains("/talker") && text.contains("demo/talker"),
        "{text}"
    );
    assert_eq!(
        text,
        run(&[&base[..], &["--print-graph"]].concat(), &[]).1,
        "old flag prints the same"
    );

    let (_, json, _) = run(
        &[&["graph"][..], &base, &["--format", "json"]].concat(),
        &[],
    );
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let g = &v["graphs"][0];
    let topic = g["topics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "/chatter")
        .expect("/chatter");
    assert_eq!(topic["type"], "std_msgs::msg::String");
    assert_eq!(topic["publishers"].as_array().unwrap().len(), 1);
    assert_eq!(topic["subscribers"].as_array().unwrap().len(), 1);

    let (_, dot, _) = run(&[&["graph"][..], &base, &["--format", "dot"]].concat(), &[]);
    assert!(
        dot.starts_with("digraph rosfix {") && dot.trim_end().ends_with('}'),
        "{dot}"
    );
    assert!(
        dot.contains("shape=ellipse, label=\"/chatter\\nstd_msgs::msg::String\""),
        "{dot}"
    );
    // A quote in a node name is escaped, not left to break the DOT file.
    assert!(dot.contains("say \\\"hi\\\""), "{dot}");
    assert_eq!(dot.matches('{').count(), dot.matches('}').count());

    let (_, mm, _) = run(
        &[&["graph"][..], &base, &["--format", "mermaid"]].concat(),
        &[],
    );
    assert!(mm.starts_with("flowchart LR\n"));
    assert!(mm.contains("([\"/chatter\"])"), "{mm}");
    assert!(mm.contains("#quot;hi#quot;"), "{mm}");
    assert_eq!(
        mm.matches("subgraph").count(),
        mm.matches("\n  end\n").count()
    );

    let out = ws.path().join("g.dot");
    run(
        &[&["graph"][..], &base, &["--format", "dot", "-o", p(&out)]].concat(),
        &[],
    );
    assert_eq!(std::fs::read_to_string(out).unwrap(), dot);

    // The callback graph: the talker's timer with its period, the listener's subscription
    // and the edge from the talker's publisher to that callback.
    let (_, cb, _) = run(
        &[&["graph"][..], &base, &["--format", "callbacks"]].concat(),
        &[],
    );
    let v: serde_json::Value = serde_json::from_str(&cb).unwrap();
    assert_eq!(v["format"], "rosfix-callback-graph");
    let g = &v["graphs"][0];
    let node = |name: &str| {
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["name"].as_str().unwrap().contains(name))
            .unwrap()
            .clone()
    };
    let talker = node("talker");
    let timer = &talker["callbacks"][0];
    assert_eq!(
        (timer["kind"].as_str(), timer["period_ms"].as_f64()),
        (Some("timer"), Some(250.0)),
        "{cb}"
    );
    assert_eq!(talker["publishers"][0]["topic"], "/chatter");
    let listener = node("hi");
    let sub = &listener["callbacks"][0];
    assert_eq!(
        (sub["kind"].as_str(), sub["topic"].as_str()),
        (Some("subscription"), Some("/chatter")),
        "{cb}"
    );
    let edge = &g["edges"][0];
    assert_eq!(
        (edge["from_node"].as_str(), edge["to_callback"].as_str()),
        (talker["id"].as_str(), sub["id"].as_str()),
        "{cb}"
    );
}

#[test]
fn baseline_command_hides_recorded_findings() {
    let ws = workspace();
    let file = tempfile::tempdir().unwrap();
    let b = file.path().join("b.json");
    let (code, _, err) = run(&["baseline", p(ws.path()), "--no-ros", "-o", p(&b)], &[]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("fingerprint(s)"), "{err}");
    let (code, _, err) = run(
        &[
            "check",
            p(ws.path()),
            "--no-ros",
            "--baseline",
            p(&b),
            "--fail-on",
            "info",
        ],
        &[],
    );
    assert_eq!(code, 0, "{err}");
}

#[test]
fn init_writes_valid_files_and_keeps_existing_ones() {
    let dir = tempfile::tempdir().unwrap();
    let (code, out, err) = run(
        &["init", p(dir.path()), "--distro", "jazzy", "--pre-commit"],
        &[],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("created   rosfix.toml"), "{out}");
    let toml = std::fs::read_to_string(dir.path().join("rosfix.toml")).unwrap();
    assert!(toml.contains("distro = \"jazzy\""));
    let wf = std::fs::read_to_string(dir.path().join(".github/workflows/rosfix.yml")).unwrap();
    assert!(wf.contains("uses: SohamWalam11/ros2-safety-lint@v1") && wf.contains("distro: jazzy"));
    assert!(dir.path().join(".pre-commit-config.yaml").exists());
    // The generated configuration is accepted by check.
    let (code, _, err) = run(&["check", p(dir.path()), "--no-ros"], &[]);
    assert_eq!(code, 0, "{err}");
    // A second run keeps everything.
    std::fs::write(dir.path().join("rosfix.toml"), "fail_on = \"error\"\n").unwrap();
    let (_, out, _) = run(&["init", p(dir.path())], &[]);
    assert!(out.contains("kept      rosfix.toml"), "{out}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("rosfix.toml")).unwrap(),
        "fail_on = \"error\"\n"
    );
}

#[test]
fn doctor_exit_code_follows_errors() {
    let (code, out, _) = run(&["doctor"], &[]);
    assert_eq!(code, 0, "only warnings without a ROS environment:\n{out}");
    assert!(
        out.contains("ROS_DISTRO") && out.contains("0 error(s)"),
        "{out}"
    );
    let (code, out, _) = run(
        &["doctor"],
        &[
            ("ROS_DISTRO", "jazzy"),
            ("RMW_IMPLEMENTATION", "rmw_cyclonedds_cp"),
        ],
    );
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("Did you mean `rmw_cyclonedds_cpp`?"), "{out}");
}

#[test]
fn migrate_needs_a_known_source_distribution() {
    let ws = workspace();
    let (code, _, err) = run(&["migrate", p(ws.path()), "--to", "jazzy"], &[]);
    assert_eq!(code, 2);
    assert!(err.contains("the current distribution is unknown"), "{err}");
    let (code, _, err) = run(
        &[
            "migrate",
            p(ws.path()),
            "--from",
            "kilted",
            "--to",
            "rolling",
        ],
        &[],
    );
    assert_eq!(code, 2);
    assert!(err.contains("same knowledge pack"), "{err}");
    // A workspace without upstream nodes has nothing new on another distribution.
    let (code, _, err) = run(
        &[
            "migrate",
            p(ws.path()),
            "--from",
            "humble",
            "--to",
            "jazzy",
            "--fail-on",
            "info",
        ],
        &[],
    );
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("humble -> jazzy: 0 new finding(s)"), "{err}");
}

#[test]
fn migrate_reports_an_upstream_rename() {
    let dir = tempfile::tempdir().unwrap();
    let files: &[(&str, &str)] = &[
        ("bringup/package.xml", "<package format=\"3\"><name>bringup</name><exec_depend>nav2_behaviors</exec_depend></package>"),
        ("bringup/launch/nav.launch.xml", "<launch><node pkg=\"nav2_behaviors\" exec=\"behavior_server\" name=\"behavior_server\"><param from=\"$(find-pkg-share bringup)/config/b.yaml\"/></node></launch>"),
        ("bringup/config/b.yaml", "behavior_server:\n  ros__parameters:\n    costmap_topic: local_costmap/costmap_raw\n"),
    ];
    for (rel, text) in files {
        let f = dir.path().join(rel);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, text).unwrap();
    }
    let (code, out, err) = run(
        &[
            "migrate",
            p(dir.path()),
            "--from",
            "humble",
            "--to",
            "jazzy",
            "--format",
            "json",
        ],
        &[],
    );
    assert_eq!(code, 1, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let d = v["diagnostics"].as_array().unwrap();
    assert!(
        d.iter()
            .any(|x| x["message"].as_str().unwrap().contains("costmap_topic")),
        "{out}"
    );
    // Going the other way, the parameter exists again.
    let (code, _, err) = run(
        &[
            "migrate",
            p(dir.path()),
            "--from",
            "jazzy",
            "--to",
            "humble",
            "--fail-on",
            "warning",
        ],
        &[],
    );
    assert!(!err.contains("costmap_topic"), "{err}");
    let _ = code;
}

#[test]
fn triage_explains_a_launch_error_with_its_finding() {
    let dir = tempfile::tempdir().unwrap();
    let launch = "from launch import LaunchDescription\nfrom launch.actions import DeclareLaunchArgument\nfrom launch.substitutions import LaunchConfiguration\nfrom launch_ros.actions import Node\ndef generate_launch_description():\n    return LaunchDescription([\n        DeclareLaunchArgument('use_sim_time', default_value='false'),\n        Node(package='demo', executable='talker', parameters=[{'use_sim_time': LaunchConfiguration('use_sim_tme')}]),\n    ])\n";
    for (rel, text) in [
        ("demo/package.xml", "<package format=\"3\"><name>demo</name><exec_depend>launch_ros</exec_depend></package>"),
        ("demo/launch/a.launch.py", launch),
    ] {
        let f = dir.path().join(rel);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, text).unwrap();
    }
    let log = dir.path().join("launch.log");
    std::fs::write(
        &log,
        "[INFO] [launch]: All log files can be found below /tmp\n[ERROR] [launch]: Caught exception in launch (see debug for traceback): launch configuration 'use_sim_tme' does not exist\n",
    )
    .unwrap();
    let (code, out, err) = run(
        &[
            "triage",
            p(&log),
            p(dir.path()),
            "--no-ros",
            "--format",
            "json",
        ],
        &[],
    );
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"));
    let items = v.as_array().unwrap();
    assert_eq!(items.len(), 1, "{out}");
    assert_eq!(items[0]["line"], 2);
    assert!(
        items[0]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule"] == "A001"),
        "{out}"
    );
    let (code, text, _) = run(&["triage", p(&log), p(dir.path()), "--no-ros"], &[]);
    assert_eq!(code, 0);
    assert!(text.contains("A001"), "{text}");
}

/// NoDL in: an installed package that registers a NoDL document for its executable gets its
/// parameters checked like a knowledge-pack entry. NoDL out: export writes valid documents.
#[test]
fn nodl_documents_in_and_out() {
    let prefix = tempfile::tempdir().unwrap();
    let pre = prefix.path();
    for (rel, text) in [
        ("share/ament_index/resource_index/packages/bat", ""),
        ("share/bat/package.xml", "<package format=\"3\"><name>bat</name></package>"),
        ("lib/bat/monitor", "#!/bin/sh\n"),
        (
            "share/ament_index/resource_index/nodl/bat__monitor",
            "nodl_version: 2\nparameters:\n  publish_rate_hz:\n    type: double\n    default_value: 1.0\n  cell_count:\n    type: int\n    default_value: 4\npublishers:\n  - name: ~/state\n    type: sensor_msgs/msg/BatteryState\n    qos: {history: KEEP_LAST, depth: 5, reliability: RELIABLE}\n",
        ),
    ] {
        let f = pre.join(rel);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, text).unwrap();
    }
    let ws = tempfile::tempdir().unwrap();
    for (rel, text) in [
        ("robot/package.xml", "<package format=\"3\"><name>robot</name><exec_depend>bat</exec_depend></package>"),
        ("robot/config/bat.yaml", "monitor:\n  ros__parameters:\n    publish_rate_hz: 2\n    cell_cuont: 6\n"),
        ("robot/launch/bat.launch.xml", "<launch><node pkg=\"bat\" exec=\"monitor\" name=\"monitor\"><param from=\"$(find-pkg-share robot)/config/bat.yaml\"/></node></launch>"),
    ] {
        let f = ws.path().join(rel);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, text).unwrap();
    }
    let (_, out, err) = run(
        &[
            "check",
            p(ws.path()),
            "--ros-prefix",
            p(pre),
            "--format",
            "json",
            "--fail-on",
            "never",
        ],
        &[],
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {err}"));
    let found: Vec<(String, String)> = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["rule"].as_str().unwrap().to_string(),
                d["message"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(
        found
            .iter()
            .any(|(r, m)| r == "P004" && m.contains("publish_rate_hz")),
        "{found:#?}"
    );
    assert!(
        found
            .iter()
            .any(|(r, m)| r == "P003" && m.contains("cell_cuont")),
        "{found:#?}"
    );

    // Out: one document per linked executable, parameters with their literal defaults.
    let src = tempfile::tempdir().unwrap();
    for (rel, text) in [
        ("talk/package.xml", "<package format=\"3\"><name>talk</name></package>"),
        ("talk/CMakeLists.txt", "project(talk)\nadd_executable(talker src/t.cpp)\ninstall(TARGETS talker DESTINATION lib/${PROJECT_NAME})\n"),
        ("talk/src/t.cpp", "#include <rclcpp/rclcpp.hpp>\n#include <std_msgs/msg/string.hpp>\nclass T : public rclcpp::Node { public: T() : Node(\"talker\") {\n  declare_parameter(\"rate\", 2.5);\n  declare_parameter<std::string>(\"frame\", \"map\");\n  declare_parameter(\"gain\", compute_gain());\n  pub_ = create_publisher<std_msgs::msg::String>(\"chatter\", rclcpp::QoS(7).reliable());\n} rclcpp::Publisher<std_msgs::msg::String>::SharedPtr pub_; };\n"),
    ] {
        let f = src.path().join(rel);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, text).unwrap();
    }
    let (code, doc, err) = run(&["nodl", "export", p(src.path()), "--no-ros"], &[]);
    assert_eq!(code, 0, "{err}");
    for want in [
        "nodl_version: 2",
        "  rate:\n    type: double\n    default_value: 2.5\n",
        "  frame:\n    type: string\n    default_value: \"map\"\n",
        // `compute_gain()`: neither the type nor the value is known, so nothing is claimed.
        "# ? parameters: gain: type not known statically",
        "publishers:\n  - name: \"chatter\"\n    type: std_msgs/msg/String\n    qos: {history: KEEP_LAST, depth: 7, reliability: RELIABLE",
    ] {
        assert!(doc.contains(want), "missing {want:?} in:\n{doc}");
    }
}

#[test]
fn check_bag_compares_the_graph_with_a_recording() {
    let ws = workspace();
    // demo: talker publishes /chatter (std_msgs/String, reliable); the launch file starts it.
    let bag = tempfile::tempdir().unwrap();
    let meta = "rosbag2_bagfile_information:\n  version: 5\n  topics_with_message_count:\n    - topic_metadata:\n        name: /chatter\n        type: std_msgs/msg/Int32\n        serialization_format: cdr\n        offered_qos_profiles: \"- history: 3\\n  depth: 0\\n  reliability: 2\\n  durability: 2\\n\"\n      message_count: 10\n    - topic_metadata:\n        name: /camera/image\n        type: sensor_msgs/msg/Image\n        serialization_format: cdr\n        offered_qos_profiles: \"\"\n      message_count: 3\n";
    std::fs::write(bag.path().join("metadata.yaml"), meta).unwrap();
    let (code, out, err) = run(
        &[
            "check-bag",
            p(bag.path()),
            p(ws.path()),
            "--no-ros",
            "--format",
            "json",
        ],
        &[],
    );
    assert_eq!(code, 1, "a type mismatch fails: {err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"));
    let kinds: Vec<(&str, &str)> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["kind"].as_str().unwrap(), i["topic"].as_str().unwrap()))
        .collect();
    assert!(kinds.contains(&("type_mismatch", "/chatter")), "{kinds:?}");
    assert!(kinds.contains(&("qos_differs", "/chatter")), "{kinds:?}");
    assert!(
        kinds.contains(&("no_static_publisher", "/camera/image")),
        "{kinds:?}"
    );

    // A matching recording passes; with --launch, a published topic that is missing is named.
    let ok = "/rosout [rcl_interfaces/msg/Log]\n";
    let list = bag.path().join("topics.txt");
    std::fs::write(&list, ok).unwrap();
    let (code, text, err) = run(
        &[
            "check-bag",
            p(&list),
            p(ws.path()),
            "--no-ros",
            "--launch",
            "demo.launch.xml",
        ],
        &[],
    );
    assert_eq!(code, 0, "{err}");
    assert!(text.contains("missing  /chatter"), "{text}");
    let (code, _, err) = run(
        &[
            "check-bag",
            p(&list),
            p(ws.path()),
            "--no-ros",
            "--launch",
            "nope.launch.py",
        ],
        &[],
    );
    assert_eq!(code, 2);
    assert!(err.contains("no launch graph"), "{err}");
}

#[test]
fn interactive_shell_runs_commands_until_exit() {
    use std::io::Write;
    let ws = workspace();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rosfix"))
        .args(["shell", p(ws.path())])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env_remove("ROS_DISTRO")
        .env_remove("AMENT_PREFIX_PATH")
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"help\nexplain P003\nrosfix rules\ncheck --no-ros --no-color --fail-on never\ncd does-not-exist\nbogus-command\ncd demo\npwd\nhistory\nexit\nexplain P004\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(out.status.success(), "{stderr}");
    assert!(
        stdout.contains("shell, workspace") && stdout.contains("Shell commands:"),
        "{stdout}"
    );
    assert!(stdout.contains("P003 (params-undeclared)"), "{stdout}");
    assert!(stdout.contains("R001  core"), "rules ran: {stdout}");
    assert!(
        stdout.contains("P004"),
        "check ran on the workspace: {stdout}"
    );
    assert!(stderr.contains("cd does-not-exist"), "{stderr}");
    assert!(
        stderr.contains("unrecognized") || stderr.contains("error"),
        "a typo does not end the shell: {stderr}"
    );
    assert!(
        stdout.lines().any(|l| l.trim_end().ends_with("demo")),
        "cd changed the workspace: {stdout}"
    );
    assert!(stdout.contains("   9  history"), "{stdout}");
    // Nothing after `exit` runs.
    assert!(!stdout.contains("P004 (params-type-mismatch)"), "{stdout}");
}

#[test]
fn docs_topics() {
    let (code, out, _) = run(&["docs"], &[]);
    assert_eq!(code, 0);
    assert!(out.contains("rosfix manual") && out.contains("    rules "));
    let (_, rules, _) = run(&["docs", "rules"], &[]);
    let (_, list, _) = run(&["rules"], &[]);
    assert_eq!(
        rules.matches("\n## ").count(),
        list.lines().count(),
        "one section per rule"
    );
    let (code, _, err) = run(&["docs", "nope"], &[]);
    assert_eq!(code, 2);
    assert!(err.contains("unknown topic `nope`"), "{err}");
}
