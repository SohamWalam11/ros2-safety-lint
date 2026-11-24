//! Static registry of every rule rosfix can report.
//!
//! Rule IDs are stable: they appear in SARIF, JSON, suppressions, baselines and config.

use serde::Serialize;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "info" | "note" => Some(Severity::Info),
            "warning" | "warn" => Some(Severity::Warning),
            "error" => Some(Severity::Error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Rule {
    pub id: &'static str,
    pub name: &'static str,
    /// The agent (domain checker) that owns this rule.
    pub agent: &'static str,
    pub severity: Severity,
    pub enabled_by_default: bool,
    pub summary: &'static str,
    pub explanation: &'static str,
}

macro_rules! rule {
    ($id:literal, $name:literal, $agent:literal, $sev:ident, $on:literal, $summary:literal, $expl:literal) => {
        Rule {
            id: $id,
            name: $name,
            agent: $agent,
            severity: Severity::$sev,
            enabled_by_default: $on,
            summary: $summary,
            explanation: $expl,
        }
    };
}

pub static RULES: &[Rule] = &[
    // ---------------------------------------------------------------- infrastructure
    rule!("R001", "unparsable-file", "core", Info, true,
        "A ROS-relevant file could not be parsed, so it was not analysed.",
        "rosfix skips files it cannot parse instead of guessing. Fix the syntax error (or exclude the file) so the other checks can see its contents."),
    // ---------------------------------------------------------------- parameters
    rule!("P001", "params-unmatched-node-key", "params", Warning, true,
        "A node key in a parameter file matches no node that loads the file, so its values are silently dropped.",
        "ROS 2 matches parameter-file keys against each node's fully-qualified name (namespace included). A key such as `controller_server` does not match `/robot1/controller_server`. Use the exact fully-qualified name, a wildcard such as `/**`, or fix the node name."),
    rule!("P002", "params-missing-ros-parameters", "params", Error, true,
        "A node section in a parameter file has no `ros__parameters` level, so none of its values are applied.",
        "Every node section must contain a `ros__parameters:` mapping (two underscores). Values placed directly under the node key, or under a misspelled key, are ignored without an error."),
    rule!("P003", "params-undeclared", "params", Warning, true,
        "A parameter is set for a node that never declares it, so the value is silently ignored.",
        "Since Dashing, ROS 2 only applies overrides for parameters that the node declares (unless it opts into undeclared parameters). This is usually a typo or a parameter that was renamed in the code."),
    rule!("P004", "params-type-mismatch", "params", Error, true,
        "A parameter value's type differs from the type the node declares; the node will throw at startup.",
        "`declare_parameter(\"x\", 1.0)` declares a double. Overriding it with `x: 1` (an integer) raises an invalid-type error at runtime. Write `1.0`."),
    rule!("P005", "params-required-missing", "params", Warning, true,
        "A node declares a parameter without a default, but no launch source provides a value.",
        "Declaring a statically typed parameter without a default (e.g. `declare_parameter<double>(\"x\")`) throws when no override is supplied."),
    rule!("P006", "params-constraint-violated", "params", Error, true,
        "A parameter value violates a constraint the node declares (range, bound, allowed values, size); the node throws at startup.",
        "rclcpp checks overrides against a `ParameterDescriptor`'s `floating_point_range`/`integer_range` when the parameter is declared, and generate_parameter_library checks its `validation:` rules (`bounds<>`, `gt<>`, `lt_eq<>`, `one_of<>`, `not_empty<>`, `size_lt<>`, `fixed_size<>`, …). rosfix also reads guards the node writes itself, such as `rate_ = declare_parameter(\"rate\", 10.0); if (rate_ <= 0.0) throw …;`, which require `rate > 0`. A violating value raises an exception and the node exits."),
    rule!("P007", "params-renamed-in-distro", "params", Warning, true,
        "A parameter is not declared by an installed node in this ROS distribution, but is in another one: it was renamed or removed.",
        "Parameter names of packages such as Nav2 and ros2_controllers change between releases (rosfix knows them per distribution from its knowledge packs). A configuration written for another release silently loses these values."),
    // ---------------------------------------------------------------- topics
    rule!("P008", "params-conflict", "params", Warning, true,
        "Two parameters of one node contradict each other, so the node ignores one of them or misbehaves.",
        "Each value is valid on its own, but together they are not: for example robot_localization requires map_frame, odom_frame and base_link_frame to differ, and a Nav2 costmap with a footprint never uses robot_radius. Only constraints that the node's own code enforces are checked; unset parameters take their declared defaults."),
    rule!("T001", "topic-near-miss", "topics", Warning, true,
        "A subscription has no matching publisher, but a publisher exists on a very similar name.",
        "After namespaces and remaps are applied, the two endpoints use different names, so they never connect. Typical causes: a typo, a missing/extra namespace, or an absolute name (`/scan`) inside a namespaced node."),
    rule!("T002", "topic-unused-remap", "topics", Warning, true,
        "A remapping rule does not match any name the node uses, so it has no effect.",
        "Remaps are matched against the node's fully expanded names. A remap of a name the node never uses is silently ignored."),
    rule!("T003", "topic-type-conflict", "topics", Error, true,
        "The same topic is used with different message types.",
        "Endpoints with different types on one topic never exchange data and produce type-mismatch warnings at runtime."),
    rule!("T004", "topic-absolute-in-namespace", "topics", Info, true,
        "A node launched in a namespace uses an absolute topic name, which escapes the namespace.",
        "Names starting with `/` ignore the node's namespace. In multi-robot setups this makes several robots share one topic. Prefer relative names."),
    rule!("T005", "topic-no-publisher", "topics", Info, false,
        "A subscription has no publisher anywhere in the analysed launch graph.",
        "Only reported when every node in the launch graph could be resolved to source code (closed world). External tools (RViz, rosbag, drivers on other machines) may still publish it."),
    // ---------------------------------------------------------------- executor
    rule!("E001", "executor-blocking-wait", "executor", Error, true,
        "A callback blocks waiting for a result that can only be delivered by the same executor thread: the node deadlocks.",
        "Waiting on a service/action future (or spinning) inside a callback blocks the executor that must deliver the response. Use an async request with a response callback, or put the client in a separate callback group and use a MultiThreadedExecutor."),
    rule!("E002", "executor-sleep-in-callback", "executor", Warning, true,
        "A callback sleeps, stalling every other callback on the same executor thread.",
        "Use a timer instead of sleeping inside a callback."),
    // ---------------------------------------------------------------- ros2_control
    rule!("C001", "control-unknown-joint", "control", Error, true,
        "A controller is configured with a joint that no `<ros2_control>` block declares.",
        "The controller fails to activate because the controller manager cannot find the joint's interfaces."),
    rule!("C002", "control-missing-interface", "control", Error, true,
        "A controller needs a command/state interface the hardware does not export.",
        "For example a velocity controller on a joint that only declares a position command interface: activation fails with `command interface does not exist`."),
    rule!("C003", "control-unknown-type", "control", Warning, true,
        "A controller `type` looks like a typo of a known controller plugin.",
        "The controller manager cannot load a plugin whose lookup name does not exist."),
    rule!("C004", "control-config-mismatch", "control", Warning, true,
        "Controller configuration is inconsistent between the controller manager and the controller's own section.",
        "Either a controller is listed in `controller_manager` without its own parameter section, or a section exists for a controller with no type."),
    rule!("C005", "control-joint-not-in-urdf", "control", Error, true,
        "A `<ros2_control>` joint does not exist in the robot description.",
        "Every joint in a `<ros2_control>` tag must exist as a URDF `<joint>` (enforced since Jazzy)."),
    rule!("C006", "control-missing-hardware-plugin", "control", Error, true,
        "A `<ros2_control>` block has no hardware plugin.",
        "The resource manager cannot load hardware without `<hardware><plugin>…</plugin></hardware>`."),
    rule!("C007", "control-spawned-not-defined", "control", Error, true,
        "A spawner starts a controller that its controller manager does not define.",
        "`spawner` loads a controller by name; without `<name>.type` in the controller manager's parameters (or `--param-file` / `--controller-type` on the spawner) loading fails and the controller never runs."),
    rule!("C008", "control-defined-not-spawned", "control", Warning, true,
        "A controller is configured for a controller manager, but no spawner in the workspace starts it.",
        "Configuring a controller does not load it: some spawner (or service call) must load and activate it. Usually the name is missing from a spawner, or a spawner has a typo."),
    rule!("C009", "control-param-constraint", "control", Error, true,
        "A controller parameter violates a validator of the controller's parameter specification.",
        "ros2_controllers declare their parameters with generate_parameter_library; values outside `validation:` rules (`gt<>`, `bounds<>`, `one_of<>`, `not_empty<>`, …) make the controller fail in `on_configure`, so it is never activated."),
    rule!("C010", "control-update-rate", "control", Warning, true,
        "A controller's `update_rate` is higher than the controller manager's, or does not divide it.",
        "The controller manager calls controllers from its own loop: a controller cannot run faster than `controller_manager.update_rate`, and a rate that does not divide it is only approximated (the manager skips cycles). Use a divisor of the manager's rate."),
    // ---------------------------------------------------------------- launch / packaging
    rule!("L001", "launch-unknown-executable", "packaging", Error, true,
        "A launch file starts an executable that the package does not build or install.",
        "`ros2 launch` fails with `executable not found`. Check the name against `add_executable`/`install(TARGETS …)` or `console_scripts`."),
    rule!("L002", "package-missing-dependency", "packaging", Warning, true,
        "A package uses another ROS package that is not declared in package.xml.",
        "Builds may pass locally but break on clean machines, in CI, or for binary releases."),
    rule!("L003", "python-setup-cfg", "packaging", Error, true,
        "An ament_python package installs console scripts but `setup.cfg` does not put them in `lib/<package>`.",
        "Without `[install] install_scripts=$base/lib/<pkg>` (and the matching `[develop]` entry) `ros2 run`/`ros2 launch` cannot find the executables."),
    rule!("L004", "package-uninstalled-files", "packaging", Warning, true,
        "Launch or config files exist but are never installed into the package's share directory.",
        "`ros2 launch <pkg> <file>` and `get_package_share_directory()` only see installed files."),
    rule!("L005", "launch-missing-file", "packaging", Error, true,
        "A launch file references a parameter/description file that does not exist.",
        "The path resolves to a file that is not present in the package."),
    rule!("L006", "component-macro-missing", "packaging", Error, true,
        "A class is registered as a component in CMake, but no source file registers it with `RCLCPP_COMPONENTS_REGISTER_NODE`.",
        "`rclcpp_components_register_nodes` only adds the class name to the ament index; the class loader also needs the factory that `RCLCPP_COMPONENTS_REGISTER_NODE(<class>)` creates in the library. Without it, loading the component fails."),
    rule!("L007", "interfaces-not-generated", "packaging", Warning, true,
        "Interface files and `rosidl_generate_interfaces` disagree: a `.msg`/`.srv`/`.action` file is not generated, or a listed file does not exist.",
        "Only files listed in `rosidl_generate_interfaces` get C++/Python code; a missing one breaks every package that uses the type, and a listed file that does not exist fails CMake configuration."),
    rule!("L008", "interfaces-group-missing", "packaging", Error, true,
        "A package generates interfaces but is not a member of `rosidl_interface_packages`.",
        "rosidl requires `<member_of_group>rosidl_interface_packages</member_of_group>` in package.xml of every package that calls `rosidl_generate_interfaces`; CMake configuration fails without it."),
    rule!("L009", "package-unknown-rosdep-key", "packaging", Warning, true,
        "A package.xml dependency is not a rosdep key, a released package of the ROS distribution, or a workspace package.",
        "`rosdep install --from-paths src` fails with `Cannot locate rosdep definition for [x]`, so the workspace cannot be set up on a fresh machine or in CI. Usually a typo, a renamed package, or a dependency that only exists in another workspace (then add it there or use `--skip-keys`). Checked against rosdistro and rosdep keys embedded per distribution."),
    rule!("L010", "component-library-not-installed", "packaging", Error, true,
        "A library whose classes are registered as components is never installed.",
        "The component index points at the library, but without `install(TARGETS <lib> …)` it only exists in the build space: loading the component from the install space fails (it may still work with `--symlink-install` or from the build tree, which hides the bug)."),
    rule!("L011", "install-source-missing", "packaging", Error, true,
        "An install(FILES|PROGRAMS|DIRECTORY …) rule names a path that does not exist.",
        "CMake fails the install step with \"file INSTALL cannot find\", so `colcon build` fails for the package (often only after a file was renamed or a directory removed). Only literal paths are checked; paths built from variables rosfix cannot expand, generator expressions and OPTIONAL rules are skipped."),
    rule!("L012", "python-subpackage-not-installed", "packaging", Warning, true,
        "setup.py lists its packages by hand and leaves out a subpackage that exists.",
        "setuptools installs exactly the packages listed in `packages=`. A subpackage (a directory with `__init__.py`) missing from the list is not installed, so `import pkg.sub` fails from the install space while it works from the source tree. `find_packages()` lists them automatically."),
    // ---------------------------------------------------------------- sim time
    rule!("S001", "simtime-mixed", "simtime", Warning, true,
        "Nodes in one launch graph disagree on `use_sim_time`.",
        "Mixing simulated and wall-clock time produces tf `extrapolation into the future/past` errors and stalled timers."),
    rule!("S002", "simtime-missing-with-simulator", "simtime", Warning, true,
        "A simulator is launched but some nodes do not use simulated time.",
        "When a simulator publishes `/clock`, every node should set `use_sim_time: true`."),
    rule!("S003", "simtime-no-clock", "simtime", Info, false,
        "Nodes use simulated time but nothing in the launch graph publishes `/clock`.",
        "Time stays at zero until a clock source (simulator or bag with --clock) runs."),
    // ---------------------------------------------------------------- QoS
    rule!("Q001", "qos-reliability-incompatible", "qos", Error, true,
        "A RELIABLE subscription is connected to a BEST_EFFORT publisher: no messages are delivered.",
        "DDS requires offered reliability ≥ requested. Make the subscription BEST_EFFORT (e.g. `SensorDataQoS`) or the publisher RELIABLE."),
    rule!("Q002", "qos-durability-incompatible", "qos", Error, true,
        "A TRANSIENT_LOCAL subscription is connected to a VOLATILE publisher: no messages are delivered.",
        "DDS requires offered durability ≥ requested."),
    rule!("Q003", "qos-deadline-liveliness-incompatible", "qos", Error, true,
        "Publisher and subscription deadline or liveliness settings are incompatible.",
        "The offered deadline must be ≤ the requested deadline; the offered liveliness kind must be ≥ requested and its lease ≤ requested."),
    rule!("Q004", "qos-late-joiner", "qos", Info, true,
        "A VOLATILE subscription listens to a latched (TRANSIENT_LOCAL) topic and will miss messages published before it started.",
        "Topics such as `/map` and `/robot_description` are published once. Subscribe with TRANSIENT_LOCAL durability."),
    rule!("Q005", "qos-override-ignored", "qos", Warning, true,
        "A `qos_overrides` parameter targets an endpoint that does not accept QoS overrides, so it is ignored.",
        "QoS overrides only apply when the publisher/subscription is created with `QosOverridingOptions`. Otherwise the YAML silently has no effect — the code's QoS wins."),
    // ---------------------------------------------------------------- Nav2
    rule!("N001", "nav2-plugin-section-missing", "nav2", Error, true,
        "A plugin listed in a Nav2 plugin list has no parameter section defining it.",
        "Each name in e.g. `controller_plugins` or a costmap's `plugins` needs a sibling section with a `plugin:` type."),
    rule!("N002", "nav2-unknown-plugin", "nav2", Warning, true,
        "A Nav2 `plugin:` type looks like a typo of a known plugin.",
        "pluginlib cannot load an unknown lookup name; the server fails to configure."),
    rule!("N003", "nav2-unknown-frame", "nav2", Warning, true,
        "A Nav2 frame parameter names a frame that does not exist in the robot description.",
        "tf lookups for the frame will fail at runtime."),
    rule!("N004", "nav2-footprint", "nav2", Error, true,
        "A costmap footprint is empty, degenerate, or has a non-positive robot radius.",
        "Collision checking uses the footprint; an invalid one makes the robot effectively a point."),
    rule!("N005", "nav2-inflation-too-small", "nav2", Warning, true,
        "The inflation radius is smaller than the robot's footprint radius.",
        "Planned paths can then hug obstacles closer than the robot is wide."),
    rule!("N006", "nav2-unknown-param", "nav2", Warning, true,
        "A Nav2 parameter name looks like a typo of a known parameter for that plugin.",
        "Unknown names are ignored, so the intended setting never applies."),
    rule!("N008", "nav2-plugin-renamed", "nav2", Error, true,
        "A Nav2 plugin name does not exist in this ROS distribution but does in another (or is a known pre-Humble name): it was renamed or removed.",
        "pluginlib cannot load the class (`class … does not exist`), so the server fails to configure and lifecycle bringup aborts. Configurations copied from tutorials for another release are the usual cause, e.g. AMCL `robot_model_type: differential` instead of `nav2_amcl::DifferentialMotionModel`."),
    rule!("N009", "nav2-bt-unknown-node", "nav2", Warning, true,
        "A behavior-tree XML uses a node type that BehaviorTree.CPP, the distribution's Nav2 packages and the workspace do not provide (a typo, or a node of another release).",
        "`bt_navigator` loads the tree at activation; an unknown node type fails with `Error at line …: Node not recognized`, the server does not activate and bringup aborts. Types from custom plugin libraries that rosfix cannot see are not reported."),
    rule!("N010", "nav2-bt-unknown-subtree", "nav2", Error, true,
        "`<SubTree ID>` refers to a behavior tree that no tree file defines, and is a near-miss of one that is defined (typo).",
        "BehaviorTree.CPP resolves subtrees by ID when the tree is created; an unknown ID fails tree creation. IDs that resemble no tree in the workspace are not reported: bt_navigator also loads trees from installed search directories."),
    // ---------------------------------------------------------------- launch arguments
    rule!("A001", "launch-arg-undeclared", "launch", Warning, true,
        "A launch configuration is used without a default, but no launch file that runs this one declares it.",
        "`LaunchConfiguration('x')` / `$(var x)` fails with \"launch configuration 'x' does not exist\" unless `x` is declared (`DeclareLaunchArgument`, `<arg>`), passed by an including file, or given on the command line. Usually a typo of a declared argument."),
    rule!("A002", "launch-arg-unused", "launch", Info, true,
        "A launch argument is declared but never used in its file or the files it includes.",
        "Setting it on the command line changes nothing. Either the option is dead or its use has a typo."),
    rule!("A003", "launch-duplicate-node", "launch", Warning, true,
        "Two nodes in one launch graph get the same fully qualified name.",
        "ROS 2 does not enforce unique node names, but two nodes with one name share parameter services, overwrite each other's entries in `ros2 node list` and `ros2 param`, and receive each other's parameter-file sections. Give one of them another `name` or `namespace`."),
    // ---------------------------------------------------------------- middleware / network
    rule!("D001", "net-domain-id-invalid", "network", Error, true,
        "`ROS_DOMAIN_ID` is not an integer from 0 to 232.",
        "The domain ID selects the DDS discovery ports; values that are not integers, or above 232, make participant creation fail. Use 0–101 or 215–232 on Linux."),
    rule!("D002", "net-rmw-unknown", "network", Error, true,
        "`RMW_IMPLEMENTATION` names no known middleware (likely a typo).",
        "rcl loads the RMW library by this name; an unknown name aborts every node at startup."),
    rule!("D003", "net-dds-config-missing", "network", Error, true,
        "A middleware configuration file (`CYCLONEDDS_URI`, `FAST*_DEFAULT_PROFILES_FILE`, `ZENOH_*_CONFIG_URI`) does not exist or is not valid XML.",
        "The middleware then fails to start or silently runs with defaults (other domain, transports or QoS than intended). rmw_zenoh_cpp stops initializing with \"Invalid configuration file\"."),
    rule!("D004", "net-localhost-only-deprecated", "network", Info, true,
        "`ROS_LOCALHOST_ONLY` is deprecated since Jazzy.",
        "Use `ROS_AUTOMATIC_DISCOVERY_RANGE=LOCALHOST` (and `ROS_STATIC_PEERS`); `ROS_LOCALHOST_ONLY` is ignored in newer releases."),
    rule!("D005", "net-zenoh-value-invalid", "network", Warning, true,
        "An rmw_zenoh_cpp environment variable has a value it cannot use.",
        "`ZENOH_CONFIG_OVERRIDE` takes `key=value` pairs separated by `;`, each value in JSON5: `mode=\"peer\"`, not `mode=peer`. A pair that is not JSON5 is ignored with a warning and one without `=` without any message. `ZENOH_ROUTER_CHECK_ATTEMPTS` that is not a number is read as 0, which waits for a router forever. Shared-memory sizes that are not positive numbers, or a message size threshold that is not a power of 2, are ignored."),
    // ---------------------------------------------------------------- lifecycle
    rule!("M001", "lifecycle-unmanaged-node", "lifecycle", Warning, true,
        "A lifecycle node is launched next to a lifecycle manager that does not list it, so it is never configured or activated.",
        "Lifecycle (managed) nodes start in the `unconfigured` state and publish nothing until a transition is requested. A `lifecycle_manager` only drives the nodes in its `node_names`. Add the node there (or give it its own `autostart`)."),
    rule!("M002", "lifecycle-unknown-managed-node", "lifecycle", Error, true,
        "A lifecycle manager's `node_names` lists a node that is not launched; bringup stops there.",
        "The manager transitions nodes in order and waits for each one's lifecycle services. A misspelled or missing node blocks bringup of every node after it."),
    // ---------------------------------------------------------------- tf
    rule!("F001", "tf-two-parents", "tf", Error, true,
        "A frame gets two different parents in one launch graph (robot description and/or static transforms).",
        "A tf frame has exactly one parent. With two publishers the tree flips between them or tf2 rejects one, and lookups through the frame fail or jump."),
    rule!("F002", "tf-static-arguments", "tf", Error, true,
        "A `static_transform_publisher` gets arguments it cannot parse, so it exits at startup.",
        "Use the flag form `--x .. --yaw .. --frame-id <parent> --child-frame-id <child>` (or the deprecated positional form: 6 or 7 numbers, then parent and child)."),
    rule!("F003", "tf-leading-slash", "tf", Error, true,
        "A frame used for tf lookups starts with `/`.",
        "tf2 in ROS 2 strips a leading `/` from transforms it receives but rejects it in lookups (a ROS 1 convention), so every lookup with it fails. Only lookup frames are reported (e.g. `global_frame`, `odom_frame`); publisher-side ids such as a static broadcaster's `frame_id` work. Remove the slash."),
    rule!("F004", "tf-unknown-frame", "tf", Warning, true,
        "A frame parameter names no known frame but is close to one (likely a typo).",
        "Known frames come from the robot description and static transforms. Lookups with a misspelled frame fail at runtime."),
    rule!("F005", "tf-cycle", "tf", Error, true,
        "The static transform tree contains a cycle (a frame is its own ancestor).",
        "tf2 requires the frames to form a tree. A static transform or URDF joint that closes a loop, including one whose parent and child are the same frame, makes lookups across the loop fail."),
    // ---------------------------------------------------------------- SROS2
    rule!("X001", "sros2-governance-weak", "security", Warning, true,
        "The DDS-Security governance allows unauthenticated participants or disables protection.",
        "Check allow_unauthenticated_participants, enable_join_access_control and the *_protection_kind settings."),
    rule!("X002", "sros2-permissions-broad", "security", Warning, true,
        "A permissions document grants access by default or through wildcards.",
        "`<default>ALLOW</default>` or `*` in allow rules give the participant access to everything."),
    rule!("X003", "sros2-permissions-expired", "security", Error, true,
        "A permissions document's validity period has ended (or not yet started).",
        "Participants with expired permissions are rejected by DDS-Security."),
    rule!("X004", "sros2-enclave-unknown", "security", Warning, true,
        "A launch file uses an enclave that no policy/keystore in the workspace defines.",
        "The node will fail to start with security enforced."),
    rule!("X005", "sros2-policy-graph-mismatch", "security", Warning, true,
        "A node publishes or subscribes to a topic its security policy does not allow.",
        "With enforcement on, the endpoint is denied at runtime."),
    rule!("X006", "sros2-keystore-absolute-path", "security", Info, true,
        "ROS_SECURITY_KEYSTORE is set to an absolute path in a launch file.",
        "Absolute keystore paths do not work on other machines or containers."),
    // ---------------------------------------------------------------- MoveIt
    rule!("V001", "moveit-kinematics-unknown-group", "moveit", Error, true,
        "A kinematics.yaml group is not a planning group of the SRDF, so its solver is never loaded.",
        "MoveIt loads one kinematics solver per SRDF planning group, looked up by the group name. A block under any other name is valid YAML but ignored, and planning reports \"No kinematics plugins defined\" for the real group."),
    rule!("V002", "moveit-joint-limits-unknown-joint", "moveit", Warning, true,
        "A joint_limits.yaml entry names a joint the robot description does not have; its limits are never used.",
        "MoveIt applies joint_limits.yaml entries by joint name on top of the URDF limits. An entry for a misspelled or renamed joint is ignored, so the planner keeps the URDF limits."),
    rule!("V003", "moveit-joint-limit-integer", "moveit", Error, true,
        "A joint limit in joint_limits.yaml is written as an integer, but MoveIt reads it as a double.",
        "MoveIt reads max_position and min_position, and max_velocity, max_acceleration and max_jerk when the matching has_*_limits flag is true, as double parameters. `2` instead of `2.0` makes it log \"expected [double] got [integer]\" and skip that limit and the joint's limits read after it, so the planner keeps the URDF values. A limit whose flag is false is not read and not reported."),
    rule!("V004", "moveit-srdf-unknown-name", "moveit", Error, true,
        "The SRDF names a joint or link the robot description does not have.",
        "Planning groups, chains, disabled collision pairs, end effectors and passive joints must use the URDF's names; MoveIt fails to build the robot model when one is missing."),
    rule!("V005", "moveit-controller-undefined", "moveit", Warning, true,
        "moveit_controllers.yaml lists a controller that no ros2_control configuration in the workspace defines.",
        "MoveIt's simple controller manager sends trajectories to the controllers it lists. A controller ros2_control does not run makes trajectory execution fail. Reported only when the workspace has ros2_control controller configuration."),
    // ---------------------------------------------------------------- robot description physics
    rule!("U001", "urdf-model-rejected", "urdf", Error, true,
        "The robot description has an element urdfdom rejects, so the whole model fails to load.",
        "urdfdom refuses a revolute or prismatic joint without `<limit>`, a `<limit>` without `effort` or `velocity`, and an `<inertial>` without `<mass value>` or with an `<inertia>` missing any of ixx, ixy, ixz, iyy, iyz, izz. robot_state_publisher, MoveIt and Gazebo then fail with \"Failed to parse robot description\"; the error names the joint or link only in the log."),
    rule!("U002", "urdf-inertia-invalid", "urdf", Warning, true,
        "A moving link's mass or inertia is not physically possible.",
        "A mass of 0 or less, an inertia tensor that is not positive definite (all zeros, a zero or negative moment), or principal moments where one exceeds the sum of the other two cannot belong to a rigid body. Physics engines reject such a link or simulate it unstably: the robot jitters, sinks or flies away in Gazebo. Checked for links carried by revolute, continuous, prismatic, floating or planar joints; links on fixed joints are merged into their parent, so their values do not matter."),
    rule!("U003", "urdf-mesh-missing", "urdf", Error, true,
        "A `package://` mesh of a workspace package does not exist, or exists only with different letter case.",
        "RViz and Gazebo look the mesh up by exact path. A missing file leaves the link without its visual or collision geometry (\"Could not load resource\"). A path that differs only in letter case works on Windows and macOS and fails on Linux. Not reported when a file of that name exists elsewhere in the package, since install rules can move it."),
    rule!("U004", "urdf-joint-limit-range", "urdf", Warning, true,
        "A revolute or prismatic joint's limit range is empty or zero.",
        "When `<limit>` sets neither `lower` nor `upper`, urdfdom uses 0 for both and the joint cannot move: Gazebo holds it in place and planners report every target as out of bounds. A `lower` above `upper` leaves no valid position. For a revolute joint without bounds use type=\"continuous\"."),
    // ---------------------------------------------------------------- ROS 1 leftovers
    rule!("O001", "ros1-launch-file", "ros1", Warning, true,
        "A ROS 2 package contains a ROS 1 XML launch file.",
        "ROS 1 launch files use `<node type=…>`, `<rosparam>` and `$(find pkg)`, which `ros2 launch` does not accept. A leftover file is either never used or fails when someone runs it. Port it (`exec=`, `<param from=…>`, `$(find-pkg-share pkg)`) or delete it; the ROS 2 migration guide lists every change."),
    rule!("O002", "ros1-client-library", "ros1", Warning, true,
        "A ROS 2 package contains Python or C++ written against the ROS 1 client libraries.",
        "`rospy`, `roslib`, `rosbag`, `tf`, `actionlib`, `dynamic_reconfigure` and `ros/ros.h` do not exist in ROS 2: such a script fails with ModuleNotFoundError when run, and such a C++ file cannot compile. Files that also use rclpy or rclcpp, or guard the import with `except ImportError`, are not reported."),
    rule!("O003", "ros1-catkin-package", "ros1", Warning, true,
        "A catkin (ROS 1) package sits in a workspace of ROS 2 packages.",
        "colcon builds a package that depends on catkin with the catkin build type, which fails where only ROS 2 is installed, and the failure stops `colcon build` for the whole workspace. Port the package to ament, or put an empty COLCON_IGNORE file in its directory so colcon skips it."),
    // ---------------------------------------------------------------- ros_gz_bridge
    rule!("G001", "gz-bridge-unsupported-pair", "gzbridge", Error, true,
        "A ros_gz_bridge topic pairs a ROS type and a Gazebo type the bridge cannot convert between.",
        "ros_gz_bridge looks up a converter by the exact ROS type string (`pkg/msg/Name`) and Gazebo type string (`gz.msgs.Name` or `ignition.msgs.Name`). A pair it has no converter for, or a type written in another form (`std_msgs/String`, `gz::msgs::Twist`), fails with \"No template specialization for the pair\" and that topic is not bridged. Pairs are checked against the ros_gz_bridge mapping table; types outside the table are not reported."),
    rule!("G002", "gz-bridge-config-invalid", "gzbridge", Error, true,
        "A ros_gz_bridge YAML entry is rejected when the bridge starts, so its topic is not bridged.",
        "The bridge skips an entry that lacks `ros_type_name` or `gz_type_name`, sets `topic_name` together with `ros_topic_name` or `gz_topic_name`, or has a `direction` or `qos_profile` it does not know. Only an error line in the log shows it."),
    rule!("G003", "gz-bridge-argument-invalid", "gzbridge", Error, true,
        "A parameter_bridge argument does not follow `topic@ROS_TYPE@GZ_TYPE` (or `[` / `]` for one direction).",
        "parameter_bridge parses its arguments before bridging anything. A malformed argument, for example a missing Gazebo type, makes it print its usage text and exit, so none of the topics in that launch action are bridged. An argument without a direction separator is bridged with the same text as both types, which fails."),
    rule!("G004", "gz-bridge-unknown-key", "gzbridge", Warning, true,
        "A ros_gz_bridge YAML entry has a key the bridge does not read.",
        "The bridge reads a fixed set of keys and ignores the rest without a message. A misspelled key such as `ros_topic` or `direction_` is dropped, and the default (the other topic name, BIDIRECTIONAL) is used instead."),
    rule!("G005", "gz-bridge-direction", "gzbridge", Warning, true,
        "A one-way parameter_bridge topic points against how the ROS nodes use it.",
        "`]` carries a topic from ROS to Gazebo only and `[` from Gazebo to ROS only. When the ROS nodes of the launch graph only subscribe to a `]` topic, they never get Gazebo's data (a sensor bridged the wrong way); when they only publish a `[` topic, Gazebo never gets their commands. Reported only when the endpoints of every other node in the launch graph are known."),
];

pub fn find(id: &str) -> Option<&'static Rule> {
    RULES
        .iter()
        .find(|r| r.id.eq_ignore_ascii_case(id) || r.name == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_unique() {
        let mut ids: Vec<_> = RULES.iter().map(|r| r.id).collect();
        ids.sort();
        let n = ids.len();
        ids.dedup();
        assert_eq!(n, ids.len());
    }
}
