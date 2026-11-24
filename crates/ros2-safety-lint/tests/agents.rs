//! Sandbox tests for the agents: one realistic multi-package workspace with a planted bug
//! for every agent, written to a temporary directory and analysed end to end.
//!
//! Checks, per agent:
//! - its trigger (`relevant`) fires on the sandbox and it reports the planted rules;
//! - it stays off when the workspace has nothing for it;
//! - `--agents <id>` runs only that agent (plus `core`);
//! - a clean workspace produces no findings.
//!
//! Set `ROSFIX_SANDBOX_DIR=<dir>` to keep the sandbox on disk (to run the CLI on it).

use rosfix::config::Config;
use rosfix::engine::{analyze, Analysis, Options};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const LAUNCH_PY_HEAD: &str = "from launch import LaunchDescription\nfrom launch_ros.actions import Node\nfrom ament_index_python.packages import get_package_share_directory\nimport os\n";

/// The sandbox workspace: (relative path, content).
fn sandbox() -> Vec<(&'static str, String)> {
    let mut f: Vec<(&'static str, String)> = Vec::new();
    let mut add = |p: &'static str, s: &str| f.push((p, s.to_string()));

    // ---- core: a file that cannot be parsed → R001
    add(
        "broken/package.xml",
        "<package format=\"3\"><name>broken</name></package>",
    );
    add(
        "broken/config/bad.yaml",
        "node:\n  ros__parameters:\n    a: [1, 2\n",
    );

    // ---- bot: a mobile base (params, topics, executor, qos, simtime, packaging)
    add("bot/package.xml", "<?xml version=\"1.0\"?>\n<package format=\"3\">\n  <name>bot</name>\n  <depend>rclcpp</depend>\n  <depend>std_msgs</depend>\n  <depend>sensor_msgs</depend>\n  <depend>nav_msgs</depend>\n  <depend>std_srvs</depend>\n  <export><build_type>ament_cmake</build_type></export>\n</package>\n");
    add("bot/CMakeLists.txt", "cmake_minimum_required(VERSION 3.8)\nproject(bot)\nfind_package(ament_cmake REQUIRED)\nfind_package(rclcpp REQUIRED)\nfind_package(std_msgs REQUIRED)\nfind_package(sensor_msgs REQUIRED)\nfind_package(nav_msgs REQUIRED)\nfind_package(std_srvs REQUIRED)\nadd_executable(driver src/driver.cpp)\nadd_executable(lidar src/lidar.cpp)\ninstall(TARGETS driver lidar DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch config DESTINATION share/${PROJECT_NAME})\nament_package()\n");
    add(
        "bot/src/driver.cpp",
        r#"#include <rclcpp/rclcpp.hpp>
#include <sensor_msgs/msg/laser_scan.hpp>
#include <nav_msgs/msg/odometry.hpp>
#include <std_msgs/msg/string.hpp>
#include <std_srvs/srv/trigger.hpp>
class Driver : public rclcpp::Node {
public:
  Driver() : Node("driver") {
    declare_parameter<double>("max_linear_velocity", 0.5);
    declare_parameter<double>("wheel_radius", 0.1);
    rcl_interfaces::msg::ParameterDescriptor accel;
    accel.floating_point_range.resize(1);
    accel.floating_point_range[0].from_value = 0.0;
    accel.floating_point_range[0].to_value = 2.0;
    declare_parameter("max_accel", 1.0, accel);
    client_ = create_client<std_srvs::srv::Trigger>("reset");
    scan_sub_ = create_subscription<sensor_msgs::msg::LaserScan>("scan", rclcpp::QoS(10).reliable(),
      [this](sensor_msgs::msg::LaserScan::SharedPtr) {
        auto fut = client_->async_send_request(std::make_shared<std_srvs::srv::Trigger::Request>());
        fut.wait();
      });
    odom_sub_ = create_subscription<nav_msgs::msg::Odometry>("/odom", 10,
      [this](nav_msgs::msg::Odometry::SharedPtr) {});
    status_pub_ = create_publisher<std_msgs::msg::String>("status", 10);
    timer_ = create_wall_timer(std::chrono::milliseconds(100), [this]() {
      rclcpp::sleep_for(std::chrono::milliseconds(50));
    });
  }
  rclcpp::Client<std_srvs::srv::Trigger>::SharedPtr client_;
  rclcpp::Subscription<sensor_msgs::msg::LaserScan>::SharedPtr scan_sub_;
  rclcpp::Subscription<nav_msgs::msg::Odometry>::SharedPtr odom_sub_;
  rclcpp::Publisher<std_msgs::msg::String>::SharedPtr status_pub_;
  rclcpp::TimerBase::SharedPtr timer_;
};
int main(int argc, char** argv) { rclcpp::init(argc, argv); rclcpp::spin(std::make_shared<Driver>()); }
"#,
    );
    add(
        "bot/src/lidar.cpp",
        r#"#include <rclcpp/rclcpp.hpp>
#include <sensor_msgs/msg/laser_scan.hpp>
#include <nav_msgs/msg/odometry.hpp>
#include <std_msgs/msg/bool.hpp>
class Lidar : public rclcpp::Node {
public:
  Lidar() : Node("lidar") {
    scan_pub_ = create_publisher<sensor_msgs::msg::LaserScan>("scan", rclcpp::SensorDataQoS());
    odom_pub_ = create_publisher<nav_msgs::msg::Odometry>("odom", 10);
    status_pub_ = create_publisher<std_msgs::msg::Bool>("status", 10);
  }
  rclcpp::Publisher<sensor_msgs::msg::LaserScan>::SharedPtr scan_pub_;
  rclcpp::Publisher<nav_msgs::msg::Odometry>::SharedPtr odom_pub_;
  rclcpp::Publisher<std_msgs::msg::Bool>::SharedPtr status_pub_;
};
int main(int argc, char** argv) { rclcpp::init(argc, argv); rclcpp::spin(std::make_shared<Lidar>()); }
"#,
    );
    add("bot/config/driver.yaml", "/**/driver:\n  ros__parameters:\n    max_lin_vel: 1.0\n    wheel_radius: 1\n    max_accel: 9.0\ncontroler:\n  ros__parametres:\n    x: 1\n");
    // Key `lidar` while the node runs as /robot1/lidar → P001 (values silently dropped).
    add(
        "bot/config/lidar.yaml",
        "lidar:\n  ros__parameters:\n    frame_id: laser\n",
    );
    add(
        "bot/launch/bringup.launch.py",
        &format!(
            "{LAUNCH_PY_HEAD}def generate_launch_description():\n    cfg = os.path.join(get_package_share_directory('bot'), 'config', 'driver.yaml')\n    lcfg = os.path.join(get_package_share_directory('bot'), 'config', 'lidar.yaml')\n    return LaunchDescription([\n        Node(package='bot', executable='driver', namespace='robot1', parameters=[cfg, {{'use_sim_time': True}}],\n             remappings=[('cmd_vel_typo', '/cmd_vel')]),\n        Node(package='bot', executable='lidar', namespace='robot1', parameters=[lcfg, {{'use_sim_time': False}}]),\n        Node(package='bot', executable='drivr'),\n        Node(package='gazebo_ros', executable='spawn_entity.py'),\n    ])\n"
        ),
    );

    // No simulator here: mixed clocks are S001 (with a simulator they are reported as S002).
    add(
        "bot/launch/replay.launch.py",
        &format!(
            "{LAUNCH_PY_HEAD}def generate_launch_description():\n    return LaunchDescription([\n        Node(package='bot', executable='driver', parameters=[{{'use_sim_time': True}}]),\n        Node(package='bot', executable='lidar', parameters=[{{'use_sim_time': False}}]),\n    ])\n"
        ),
    );

    // ---- arm: URDF + ros2_control + controller configuration (control)
    add(
        "arm/package.xml",
        "<package format=\"3\"><name>arm</name></package>",
    );
    add(
        "arm/urdf/arm.urdf",
        r#"<robot name="arm"><link name="base"/><link name="w"/>
<joint name="wheel_left" type="continuous"><parent link="base"/><child link="w"/></joint>
<ros2_control name="sys" type="system"><hardware><plugin>mock_components/GenericSystem</plugin></hardware>
<joint name="wheel_left"><command_interface name="position"/><state_interface name="position"/></joint>
<joint name="ghost"><command_interface name="position"/></joint>
</ros2_control></robot>"#,
    );
    // ---- a gripper description with physics mistakes (urdf: U001-U004)
    add(
        "arm/urdf/gripper.urdf",
        r#"<robot name="gripper"><link name="palm"><inertial><mass value="0.5"/><inertia ixx="0.01" iyy="0.01" izz="0.01"/></inertial>
<visual><geometry><mesh filename="package://arm/meshes/palm.stl"/></geometry></visual></link>
<link name="finger"><inertial><mass value="0"/><inertia ixx="0.001" ixy="0" ixz="0" iyy="0.001" iyz="0" izz="0.001"/></inertial></link>
<link name="thumb"/>
<joint name="finger_joint" type="prismatic"><parent link="palm"/><child link="finger"/><limit effort="5" velocity="0.1"/></joint>
<joint name="thumb_joint" type="revolute"><parent link="palm"/><child link="thumb"/></joint>
</robot>"#,
    );
    add("arm/config/controllers.yaml", "controller_manager:\n  ros__parameters:\n    vel_ctrl:\n      type: velocity_controllers/JointGroupVelocityController\n    jtc:\n      type: joint_trajectory_controler/JointTrajectoryController\nvel_ctrl:\n  ros__parameters:\n    joints: [wheel_left, wheel_rigth]\n    gain: -2.0\n    update_rate: 500\n");

    // ---- velocity_controllers: a controller package with a parameter spec (control: C009)
    add("velocity_controllers/package.xml", "<package format=\"3\"><name>velocity_controllers</name></package>");
    add("velocity_controllers/CMakeLists.txt", "project(velocity_controllers)\ngenerate_parameter_library(vc_params src/params.yaml)\n");
    add("velocity_controllers/src/params.yaml", "velocity_controllers:\n  gain: {\n    type: double,\n    default_value: 1.0,\n    validation: {\n      gt<>: [0.0]\n    }\n  }\n");

    // ---- nav: Nav2 configuration (nav2)
    add(
        "nav/package.xml",
        "<package format=\"3\"><name>nav</name></package>",
    );
    add(
        "nav/urdf/robot.urdf",
        "<robot name=\"r\"><link name=\"base_link\"/></robot>",
    );
    add(
        "nav/config/nav2.yaml",
        r#"
controller_server:
  ros__parameters:
    controller_frequncy: 20.0
    controller_plugins: ["FollowPath", "Missing"]
    FollowPath:
      plugin: "nav2_regulated_pure_pursuit_controller::RegulatedPurePursuitControler"
      desired_linear_vel: 0.5
local_costmap:
  local_costmap:
    ros__parameters:
      global_frame: odom
      robot_base_frame: base_lnk
      robot_radius: 0.3
      plugins: ["inflation_layer"]
      inflation_layer:
        plugin: "nav2_costmap_2d::InflationLayer"
        inflation_radius: 0.1
        cost_scalling_factor: 3.0
"#,
    );

    // ---- sec: SROS2 governance / permissions / policy (security)
    add(
        "sec/package.xml",
        "<package format=\"3\"><name>sec</name></package>",
    );
    add("sec/keys/governance.xml", "<dds><domain_access_rules><domain_rule><allow_unauthenticated_participants>true</allow_unauthenticated_participants></domain_rule></domain_access_rules></dds>");
    add("sec/keys/permissions.xml", "<dds><permissions><grant name=\"g\"><subject_name>CN=/t</subject_name><validity><not_before>2020-01-01T00:00:00</not_before><not_after>2021-01-01T00:00:00</not_after></validity><allow_rule><publish><topics><topic>*</topic></topics></publish></allow_rule><default>DENY</default></grant></permissions></dds>");
    add("sec/keys/policy.xml", "<policy version=\"0.2.0\"><enclaves><enclave path=\"/demo/talker\"><profiles><profile ns=\"/\" node=\"talker\"><topics publish=\"ALLOW\"><topic>chatter</topic></topics></profile></profiles></enclave></enclaves></policy>");
    add(
        "sec/launch/secure.launch.xml",
        "<launch><node pkg=\"bot\" exec=\"lidar\" ros_args=\"--enclave /demo/talkr\"/></launch>",
    );

    // ---- tfdemo: transform tree (tf)
    // ---- arm_moveit: MoveIt configuration for the arm (moveit: V001-V005)
    add("arm_moveit/package.xml", "<package format=\"3\"><name>arm_moveit</name></package>");
    add("arm_moveit/config/arm.srdf", "<robot name=\"arm\"><group name=\"manipulator\"><joint name=\"wheel_left\"/><joint name=\"ghost_joint\"/></group></robot>");
    add("arm_moveit/config/kinematics.yaml", "manipulater:
  kinematics_solver: kdl_kinematics_plugin/KDLKinematicsPlugin
");
    add("arm_moveit/config/joint_limits.yaml", "joint_limits:
  wheel_left:
    has_velocity_limits: true
    max_velocity: 2
  wheel_lft:
    max_velocity: 1.0
");
    add("arm_moveit/config/moveit_controllers.yaml", "moveit_simple_controller_manager:
  controller_names:
    - jtc2
");

    // ---- porting: ROS 1 leftovers in a ROS 2 package, and a catkin package (ros1: O001-O003)
    add("porting/package.xml", "<package format=\"3\"><name>porting</name><buildtool_depend>ament_cmake</buildtool_depend></package>");
    add("porting/launch/old.launch", "<launch><node pkg=\"porting\" type=\"driver\" name=\"driver\"/></launch>");
    add("porting/scripts/old_node.py", "import rospy\nrospy.init_node('old')\n");
    add("legacy_driver/package.xml", "<package format=\"2\"><name>legacy_driver</name><buildtool_depend>catkin</buildtool_depend></package>");

    // ---- sim_bridge: ros_gz_bridge topics (gzbridge: G001-G004)
    add("sim_bridge/package.xml", "<package format=\"3\"><name>sim_bridge</name><exec_depend>ros_gz_bridge</exec_depend></package>");
    add("sim_bridge/config/bridge.yaml", "- ros_topic_name: /imu
  ros_type_name: sensor_msgs/msg/Imu
  gz_type_name: gz.msgs.Image
  direction: GZ_TO_ROSS
- ros_topic: /odom
  ros_type_name: nav_msgs/msg/Odometry
  gz_type_name: gz.msgs.Odometry
");
    add(
        "sim_bridge/launch/bridge.launch.py",
        &format!(
            "{LAUNCH_PY_HEAD}def generate_launch_description():\n    return LaunchDescription([\n        Node(package='ros_gz_bridge', executable='parameter_bridge', arguments=['/battery@sensor_msgs/msg/BatteryState@']),\n    ])\n"
        ),
    );

    add("tfdemo/package.xml", "<package format=\"3\"><name>tfdemo</name></package>");
    add(
        "tfdemo/launch/frames.launch.py",
        &format!(
            "{LAUNCH_PY_HEAD}def generate_launch_description():\n    return LaunchDescription([\n        Node(package='tf2_ros', executable='static_transform_publisher', arguments=['--frame-id', 'base_link', '--child-frame-id', 'laser']),\n        Node(package='tf2_ros', executable='static_transform_publisher', arguments=['--frame-id', 'odom', '--child-frame-id', 'laser']),\n        Node(package='tf2_ros', executable='static_transform_publisher', arguments=['0', '0', 'map', 'odom']),\n        Node(package='tfdemo', executable='scanner', parameters=[{{'odom_frame': '/odom', 'frame_id': '/laser', 'base_frame': 'base_lnk'}}]),\n    ])\n"
        ),
    );

    // ---- lcdemo: lifecycle nodes and their manager (lifecycle)
    add("lcdemo/package.xml", "<package format=\"3\"><name>lcdemo</name></package>");
    add("lcdemo/CMakeLists.txt", "project(lcdemo)\nadd_executable(planner src/planner.cpp)\nadd_executable(mapper src/mapper.cpp)\ninstall(TARGETS planner mapper DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch DESTINATION share/${PROJECT_NAME})\n");
    add("lcdemo/src/planner.cpp", "#include <rclcpp_lifecycle/lifecycle_node.hpp>\nclass Planner : public rclcpp_lifecycle::LifecycleNode { public: Planner() : LifecycleNode(\"planner\") {} };\n");
    add("lcdemo/src/mapper.cpp", "#include <nav2_util/lifecycle_node.hpp>\nclass Mapper : public nav2_util::LifecycleNode { public: Mapper() : LifecycleNode(\"mapper\") {} };\n");
    // `planer` is a typo (M002) and `mapper` is not listed (M001).
    add(
        "lcdemo/launch/bringup.launch.py",
        &format!(
            "{LAUNCH_PY_HEAD}def generate_launch_description():\n    return LaunchDescription([\n        Node(package='lcdemo', executable='planner', name='planner'),\n        Node(package='lcdemo', executable='mapper', name='mapper'),\n        Node(package='nav2_lifecycle_manager', executable='lifecycle_manager', name='lifecycle_manager', parameters=[{{'autostart': True, 'node_names': ['planer']}}]),\n    ])\n"
        ),
    );

    // ---- arm: spawners vs controller definitions (control: C007, C008)
    add(
        "arm/launch/control.launch.py",
        &format!(
            "{LAUNCH_PY_HEAD}def generate_launch_description():\n    cfg = os.path.join(get_package_share_directory('arm'), 'config', 'controllers.yaml')\n    return LaunchDescription([\n        Node(package='controller_manager', executable='ros2_control_node', parameters=[cfg]),\n        Node(package='controller_manager', executable='spawner', arguments=['vel_ctr']),\n    ])\n"
        ),
    );

    // ---- lcdemo: launch arguments (launch: A001 typo of a declared argument, A002 unused)
    add(
        "lcdemo/launch/args.launch.py",
        &format!(
            "{LAUNCH_PY_HEAD}from launch.actions import DeclareLaunchArgument\nfrom launch.substitutions import LaunchConfiguration\ndef generate_launch_description():\n    return LaunchDescription([\n        DeclareLaunchArgument('use_sim_time', default_value='false'),\n        DeclareLaunchArgument('robot_model', default_value='burger'),\n        Node(package='lcdemo', executable='planner', name='planner', parameters=[{{'use_sim_time': LaunchConfiguration('use_sim_tme')}}]),\n    ])\n"
        ),
    );

    // ---- comp: a component registered in CMake without RCLCPP_COMPONENTS_REGISTER_NODE (L006)
    add("comp/package.xml", "<package format=\"3\"><name>comp</name><depend>rclcpp</depend><depend>rclcpp_components</depend></package>");
    add("comp/CMakeLists.txt", "project(comp)\nfind_package(rclcpp REQUIRED)\nfind_package(rclcpp_components REQUIRED)\nadd_library(talker SHARED src/talker.cpp)\nrclcpp_components_register_nodes(talker \"comp::Talker\")\ninstall(TARGETS talker DESTINATION lib)\n");
    add("comp/src/talker.cpp", "#include <rclcpp/rclcpp.hpp>\nnamespace comp { class Talker : public rclcpp::Node { public: explicit Talker(const rclcpp::NodeOptions & o) : Node(\"talker\", o) {} }; }\n");

    // ---- ifaces: an interface package with an unlisted .msg and no group membership (L007, L008)
    add("ifaces/package.xml", "<package format=\"3\"><name>ifaces</name></package>");
    add("ifaces/CMakeLists.txt", "project(ifaces)
find_package(rosidl_default_generators REQUIRED)
rosidl_generate_interfaces(${PROJECT_NAME} \"msg/Pose2.msg\")
ament_package()
");
    add("ifaces/msg/Pose2.msg", "float64 x
float64 y
");
    add("ifaces/msg/Status.msg", "uint8 code
");

    // ---- netcfg: middleware environment (network: D001 bad domain, D002 RMW typo, D003 missing DDS file)
    add("netcfg/package.xml", "<package format=\"3\"><name>netcfg</name></package>");
    add(
        "netcfg/launch/net.launch.py",
        &format!(
            "{LAUNCH_PY_HEAD}from launch.actions import SetEnvironmentVariable
def generate_launch_description():
    return LaunchDescription([
        SetEnvironmentVariable('ROS_DOMAIN_ID', 'robot1'),
        SetEnvironmentVariable('RMW_IMPLEMENTATION', 'rmw_cyclone_dds_cpp'),
        SetEnvironmentVariable('CYCLONEDDS_URI', 'file://' + os.path.join(get_package_share_directory('netcfg'), 'config', 'cyclonedds.xml')),
        SetEnvironmentVariable('ZENOH_CONFIG_OVERRIDE', 'mode=client'),
    ])
"
        ),
    );

    // ---- tools: packaging mistakes (packaging)
    add(
        "tools/package.xml",
        "<package format=\"3\">\n  <name>tools</name>\n  <depend>rclcpp</depend>\n</package>\n",
    );
    add("tools/CMakeLists.txt", "project(tools)\nfind_package(rclcpp REQUIRED)\nfind_package(sensor_msgs REQUIRED)\nadd_executable(talker src/t.cpp)\nadd_executable(hidden src/h.cpp)\ninstall(TARGETS talker DESTINATION lib/${PROJECT_NAME})\nament_package()\n");
    add("tools/launch/tools.launch.xml", "<launch><node pkg=\"tools\" exec=\"hidden\"/><node pkg=\"tools\" exec=\"talker\"><param from=\"$(find-pkg-share tools)/config/missing.yaml\"/></node></launch>");

    // ---- pyq: Python package without install_scripts (packaging)
    add("pyq/package.xml", "<package format=\"3\"><name>pyq</name><export><build_type>ament_python</build_type></export></package>");
    add("pyq/setup.py", "from setuptools import setup\nsetup(name='pyq', entry_points={'console_scripts': ['n = pyq.n:main']})\n");
    add("pyq/setup.cfg", "[metadata]\nname=pyq\n");
    f
}

/// Expected planted rules per agent.
const EXPECTED: &[(&str, &[&str])] = &[
    ("core", &["R001"]),
    ("params", &["P001", "P002", "P003", "P004", "P006"]),
    ("topics", &["T001", "T002", "T003"]),
    ("executor", &["E001", "E002"]),
    ("control", &["C001", "C002", "C003", "C005", "C007", "C008", "C009", "C010"]),
    ("tf", &["F001", "F002", "F003", "F004"]),
    ("lifecycle", &["M001", "M002"]),
    ("launch", &["A001", "A002"]),
    ("network", &["D001", "D002", "D003", "D005"]),
    ("packaging", &["L001", "L002", "L003", "L004", "L005", "L006", "L007", "L008"]),
    ("simtime", &["S001", "S002"]),
    ("qos", &["Q001"]),
    ("nav2", &["N001", "N002", "N003", "N005", "N006"]),
    ("security", &["X001", "X002", "X003", "X004"]),
    ("moveit", &["V001", "V002", "V003", "V004", "V005"]),
    ("gzbridge", &["G001", "G002", "G003", "G004"]),
    ("urdf", &["U001", "U002", "U003", "U004"]),
    ("ros1", &["O001", "O002", "O003"]),
];

fn write(dir: &Path, files: &[(&str, String)]) {
    for (rel, text) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
    }
}

fn run(dir: &Path, agents: Option<&[&str]>) -> Analysis {
    let opts = Options {
        agents: agents.map(|a| a.iter().map(|s| s.to_string()).collect()),
        ..Default::default()
    };
    analyze(dir, &Config::default(), &opts)
}

fn sandbox_dir() -> (PathBuf, Option<tempfile::TempDir>) {
    match std::env::var_os("ROSFIX_SANDBOX_DIR") {
        Some(d) => {
            let d = PathBuf::from(d);
            let _ = std::fs::remove_dir_all(&d);
            (d, None)
        }
        None => {
            let t = tempfile::tempdir().unwrap();
            (t.path().to_path_buf(), Some(t))
        }
    }
}

fn rules_by_agent(a: &Analysis) -> Vec<(String, bool, BTreeSet<&'static str>)> {
    a.runs
        .iter()
        .map(|r| {
            let rules = a
                .diagnostics
                .iter()
                .filter(|d| d.rule_info().agent == r.id)
                .map(|d| d.rule)
                .collect();
            (r.id.to_string(), r.ran, rules)
        })
        .collect()
}

#[test]
fn every_agent_triggers_and_reports_its_planted_bugs() {
    let (dir, _keep) = sandbox_dir();
    write(&dir, &sandbox());
    let a = run(&dir, None);
    let got = rules_by_agent(&a);
    for (agent, rules) in EXPECTED {
        let (_, ran, fired) = got
            .iter()
            .find(|(id, _, _)| id == agent)
            .unwrap_or_else(|| panic!("agent {agent} missing"));
        assert!(ran, "agent {agent} was not triggered");
        for r in *rules {
            assert!(
                fired.contains(r),
                "agent {agent}: expected {r}, got {fired:?}\n{:#?}",
                a.diagnostics
            );
        }
    }
    assert_eq!(
        a.runs.len(),
        EXPECTED.len(),
        "every agent is listed exactly once"
    );
}

#[test]
fn executor_messages_name_inline_callbacks_once() {
    let t = tempfile::tempdir().unwrap();
    write(t.path(), &sandbox());
    let a = run(t.path(), Some(&["executor"]));
    let msgs: Vec<&str> = a
        .diagnostics
        .iter()
        .filter(|d| d.rule.starts_with('E'))
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        msgs.iter()
            .any(|m| m.starts_with("timer callback (lambda) sleeps")),
        "{msgs:#?}"
    );
    assert!(
        msgs.iter().all(|m| !m.contains("callback callback")),
        "{msgs:#?}"
    );
}

#[test]
fn agents_stay_off_without_their_trigger() {
    let t = tempfile::tempdir().unwrap();
    // A bare package: only the packaging agent has something to look at.
    write(
        t.path(),
        &[(
            "solo/package.xml",
            "<package format=\"3\"><name>solo</name></package>".to_string(),
        )],
    );
    let a = run(t.path(), None);
    for r in &a.runs {
        assert_eq!(
            r.ran,
            r.id == "packaging",
            "agent {} ran={} on a bare package",
            r.id,
            r.ran
        );
    }
    assert!(a.diagnostics.is_empty(), "{:#?}", a.diagnostics);
}

#[test]
fn agent_selection_runs_only_the_chosen_agent() {
    let t = tempfile::tempdir().unwrap();
    write(t.path(), &sandbox());
    for (agent, _) in EXPECTED.iter().filter(|(a, _)| *a != "core") {
        let a = run(t.path(), Some(&[agent]));
        for r in &a.runs {
            if r.ran {
                assert!(
                    r.id == *agent || r.id == "core",
                    "--agents {agent} also ran {}",
                    r.id
                );
            }
        }
        assert!(
            a.diagnostics
                .iter()
                .all(|d| d.rule_info().agent == *agent || d.rule == "R001"),
            "--agents {agent}: {:#?}",
            a.diagnostics
        );
    }
}

#[test]
fn clean_workspace_has_no_findings() {
    let t = tempfile::tempdir().unwrap();
    let clean: Vec<(&str, String)> = vec![
        ("ok/package.xml", "<package format=\"3\"><name>ok</name><depend>rclcpp</depend><depend>std_msgs</depend><export><build_type>ament_cmake</build_type></export></package>".into()),
        ("ok/CMakeLists.txt", "project(ok)\nfind_package(rclcpp REQUIRED)\nfind_package(std_msgs REQUIRED)\nadd_executable(talker src/t.cpp)\nadd_executable(listener src/l.cpp)\ninstall(TARGETS talker listener DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch config DESTINATION share/${PROJECT_NAME})\nament_package()\n".into()),
        ("ok/src/t.cpp", "class T : public rclcpp::Node { public: T() : Node(\"talker\") { declare_parameter<double>(\"rate\", 1.0); pub_ = create_publisher<std_msgs::msg::String>(\"chatter\", 10); } };\n".into()),
        ("ok/src/l.cpp", "class L : public rclcpp::Node { public: L() : Node(\"listener\") { sub_ = create_subscription<std_msgs::msg::String>(\"chatter\", 10, [](std_msgs::msg::String::SharedPtr) {}); } };\n".into()),
        ("ok/config/talker.yaml", "/**/talker:\n  ros__parameters:\n    rate: 2.0\n".into()),
        ("ok/launch/demo.launch.xml", "<launch><node pkg=\"ok\" exec=\"talker\" namespace=\"r1\"><param from=\"$(find-pkg-share ok)/config/talker.yaml\"/></node><node pkg=\"ok\" exec=\"listener\" namespace=\"r1\"/></launch>".into()),
    ];
    write(t.path(), &clean);
    let a = run(t.path(), None);
    assert!(
        a.diagnostics.is_empty(),
        "false positives on a clean workspace: {:#?}",
        a.diagnostics
    );
    for id in ["params", "topics", "qos", "simtime", "packaging"] {
        assert!(
            a.runs.iter().any(|r| r.id == id && r.ran),
            "{id} should have run on the clean workspace"
        );
    }
}

/// Comparable form of a finding (file path instead of id, so two analyses line up).
fn fingerprint(a: &Analysis) -> Vec<(String, &'static str, usize, usize, String)> {
    let mut v: Vec<_> = a
        .diagnostics
        .iter()
        .map(|d| {
            (
                a.index.file(d.loc.file).rel_path.clone(),
                d.rule,
                d.loc.start,
                d.loc.end,
                d.message.clone(),
            )
        })
        .collect();
    v.sort();
    v
}

/// Differential test of the change-event dispatcher and the parse cache: for every file of
/// the sandbox and several kinds of edit, an incremental re-analysis (cache + previous run)
/// must report exactly what a fresh full analysis reports. An agent that under-declares its
/// `inputs()` would reuse stale findings and fail here.
#[test]
fn incremental_reanalysis_equals_full_analysis() {
    let t = tempfile::tempdir().unwrap();
    let files = sandbox();
    write(t.path(), &files);
    let cache = std::sync::Arc::new(rosfix::workspace::ExtractCache::default());
    let base_opts = Options {
        cache: Some(cache.clone()),
        ..Default::default()
    };
    let base = analyze(t.path(), &Config::default(), &base_opts);
    let previous = std::sync::Arc::new(base.snapshot());

    // Edits that change facts in different ways: rename identifiers, drop content, add a line.
    let edits: [fn(&str) -> String; 4] = [
        |s| s.replacen('_', "", 1),
        |s| {
            s.replacen("scan", "scam", 1)
                .replacen("joint", "jiont", 1)
                .replacen("frame", "frme", 1)
        },
        |s| {
            s.lines()
                .take(s.lines().count().saturating_sub(1))
                .collect::<Vec<_>>()
                .join("\n")
        },
        |s| format!("{s}\n"),
    ];
    let (mut checked, mut reused_agents, mut reused_files) = (0, 0, 0);
    for (rel, text) in &files {
        for edit in &edits {
            let new = edit(text);
            if new == *text {
                continue;
            }
            let mut overrides = std::collections::HashMap::new();
            overrides.insert(t.path().join(rel), new);
            let full = analyze(
                t.path(),
                &Config::default(),
                &Options {
                    overrides: overrides.clone(),
                    ..Default::default()
                },
            );
            let inc = analyze(
                t.path(),
                &Config::default(),
                &Options {
                    overrides,
                    cache: Some(cache.clone()),
                    previous: Some(previous.clone()),
                    ..Default::default()
                },
            );
            assert_eq!(
                fingerprint(&inc),
                fingerprint(&full),
                "incremental result differs after editing {rel}"
            );
            checked += 1;
            reused_agents += inc.runs.iter().filter(|r| r.reused).count();
            reused_files += inc.stats().files_reused;
        }
    }
    assert!(checked >= 60, "only {checked} edits checked");
    // The point of the exercise: work was actually skipped.
    assert!(
        reused_files > 0 && reused_agents > 0,
        "files reused {reused_files}, agents reused {reused_agents}"
    );
    eprintln!("{checked} edits: incremental == full; {reused_files} file parses and {reused_agents} agent runs skipped");
}
