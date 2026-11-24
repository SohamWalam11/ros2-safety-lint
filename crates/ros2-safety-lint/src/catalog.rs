//! Built-in knowledge used by agents: well-known controller types, Nav2 plugins and
//! parameters, and QoS of common infrastructure nodes. Catalogs are only used to
//! detect *near misses* (typos); an unknown name alone is never reported.

use crate::facts::{Durability, QosSpec, Reliability};

/// ros2_controllers plugin lookup names (Humble…Rolling) with the command
/// interface they require on their joints (`None` = configurable / not checked).
pub const CONTROLLERS: &[(&str, Option<&str>)] = &[
    ("joint_state_broadcaster/JointStateBroadcaster", None),
    (
        "joint_trajectory_controller/JointTrajectoryController",
        None,
    ),
    ("forward_command_controller/ForwardCommandController", None),
    (
        "forward_command_controller/MultiInterfaceForwardCommandController",
        None,
    ),
    (
        "position_controllers/JointGroupPositionController",
        Some("position"),
    ),
    (
        "velocity_controllers/JointGroupVelocityController",
        Some("velocity"),
    ),
    (
        "effort_controllers/JointGroupEffortController",
        Some("effort"),
    ),
    (
        "diff_drive_controller/DiffDriveController",
        Some("velocity"),
    ),
    ("tricycle_controller/TricycleController", None),
    (
        "ackermann_steering_controller/AckermannSteeringController",
        None,
    ),
    (
        "bicycle_steering_controller/BicycleSteeringController",
        None,
    ),
    (
        "tricycle_steering_controller/TricycleSteeringController",
        None,
    ),
    ("mecanum_drive_controller/MecanumDriveController", None),
    ("omni_wheel_drive_controller/OmniWheelDriveController", None),
    ("gripper_controllers/GripperActionController", None),
    ("position_controllers/GripperActionController", None),
    ("effort_controllers/GripperActionController", None),
    (
        "parallel_gripper_action_controller/GripperActionController",
        None,
    ),
    ("imu_sensor_broadcaster/IMUSensorBroadcaster", None),
    (
        "force_torque_sensor_broadcaster/ForceTorqueSensorBroadcaster",
        None,
    ),
    ("range_sensor_broadcaster/RangeSensorBroadcaster", None),
    ("pose_broadcaster/PoseBroadcaster", None),
    ("gps_sensor_broadcaster/GPSSensorBroadcaster", None),
    ("admittance_controller/AdmittanceController", None),
    ("pid_controller/PidController", None),
    ("gpio_controllers/GpioCommandController", None),
    ("chained_filter_controller/ChainedFilter", None),
    ("joint_state_controller/JointStateController", None),
];

/// Parameter names (flattened) whose values are joint names.
pub fn is_joint_param(name: &str) -> bool {
    let leaf = name.rsplit('.').next().unwrap_or(name);
    name == leaf
        && (leaf == "joints"
            || leaf == "joint"
            || leaf.ends_with("_joints")
            || leaf.ends_with("wheel_names")
            || leaf.ends_with("wheels_names")
            || leaf.ends_with("_joint_name")
            || leaf.ends_with("joints_names")
            || leaf == "command_joints")
}

/// Nav2 plugin lookup names (both `pkg::Class` and legacy `pkg/Class` forms are accepted).
pub const NAV2_PLUGINS: &[&str] = &[
    "nav2_costmap_2d::StaticLayer",
    "nav2_costmap_2d::ObstacleLayer",
    "nav2_costmap_2d::VoxelLayer",
    "nav2_costmap_2d::InflationLayer",
    "nav2_costmap_2d::RangeSensorLayer",
    "nav2_costmap_2d::DenoiseLayer",
    "nav2_costmap_2d::PluginContainerLayer",
    "nav2_costmap_2d::KeepoutFilter",
    "nav2_costmap_2d::SpeedFilter",
    "nav2_costmap_2d::BinaryFilter",
    "dwb_core::DWBLocalPlanner",
    "nav2_regulated_pure_pursuit_controller::RegulatedPurePursuitController",
    "nav2_mppi_controller::MPPIController",
    "nav2_rotation_shim_controller::RotationShimController",
    "nav2_graceful_controller::GracefulController",
    "nav2_controller::SimpleProgressChecker",
    "nav2_controller::PoseProgressChecker",
    "nav2_controller::SimpleGoalChecker",
    "nav2_controller::StoppedGoalChecker",
    "nav2_navfn_planner::NavfnPlanner",
    "nav2_smac_planner::SmacPlanner2D",
    "nav2_smac_planner::SmacPlannerHybrid",
    "nav2_smac_planner::SmacPlannerLattice",
    "nav2_theta_star_planner::ThetaStarPlanner",
    "nav2_behaviors::Spin",
    "nav2_behaviors::BackUp",
    "nav2_behaviors::DriveOnHeading",
    "nav2_behaviors::Wait",
    "nav2_behaviors::AssistedTeleop",
    "nav2_smoother::SimpleSmoother",
    "nav2_smoother::SavitzkyGolaySmoother",
    "nav2_constrained_smoother/ConstrainedSmoother",
    "nav2_constrained_smoother::ConstrainedSmoother",
    "opennav_docking::SimpleChargingDock",
    "opennav_docking::SimpleNonChargingDock",
];

pub fn normalize_plugin(s: &str) -> String {
    s.trim().replace('/', "::")
}

/// Nav2 list parameters whose entries each need a sibling section with `plugin:`.
pub const NAV2_PLUGIN_LISTS: &[&str] = &[
    "controller_plugins",
    "goal_checker_plugins",
    "progress_checker_plugins",
    "progress_checker_plugin",
    "planner_plugins",
    "behavior_plugins",
    "smoother_plugins",
    "plugins",
    "filters",
    "dock_plugins",
];

/// Known parameter names per Nav2 plugin type / server, for typo detection.
pub fn nav2_params(kind: &str) -> &'static [&'static str] {
    match kind {
        "nav2_costmap_2d::InflationLayer" => &[
            "plugin",
            "enabled",
            "inflation_radius",
            "cost_scaling_factor",
            "inflate_unknown",
            "inflate_around_unknown",
        ],
        "nav2_costmap_2d::StaticLayer" => &[
            "plugin",
            "enabled",
            "subscribe_to_updates",
            "map_subscribe_transient_local",
            "transform_tolerance",
            "map_topic",
            "footprint_clearing_enabled",
            "restore_cleared_footprint",
        ],
        "nav2_costmap_2d::ObstacleLayer" => &[
            "plugin",
            "enabled",
            "footprint_clearing_enabled",
            "max_obstacle_height",
            "min_obstacle_height",
            "combination_method",
            "observation_sources",
            "transform_tolerance",
            "tf_filter_tolerance",
        ],
        "nav2_costmap_2d::VoxelLayer" => &[
            "plugin",
            "enabled",
            "footprint_clearing_enabled",
            "max_obstacle_height",
            "min_obstacle_height",
            "z_voxels",
            "origin_z",
            "z_resolution",
            "unknown_threshold",
            "mark_threshold",
            "combination_method",
            "publish_voxel_map",
            "observation_sources",
        ],
        "costmap" => &[
            "update_frequency",
            "publish_frequency",
            "global_frame",
            "robot_base_frame",
            "use_sim_time",
            "rolling_window",
            "width",
            "height",
            "resolution",
            "robot_radius",
            "footprint",
            "footprint_padding",
            "plugins",
            "filters",
            "track_unknown_space",
            "always_send_full_costmap",
            "lethal_cost_threshold",
            "trinary_costmap",
            "unknown_cost_value",
            "transform_tolerance",
            "initial_transform_timeout",
            "map_topic",
            "map_vis_z",
            "origin_x",
            "origin_y",
            "clearable_layers",
            "use_maximum",
            "service_introspection_mode",
            "subscribe_to_stamped_footprint",
            "footprint_topic",
        ],
        "controller_server" => &[
            "controller_frequency",
            "min_x_velocity_threshold",
            "min_y_velocity_threshold",
            "min_theta_velocity_threshold",
            "failure_tolerance",
            "progress_checker_plugins",
            "progress_checker_plugin",
            "goal_checker_plugins",
            "controller_plugins",
            "use_realtime_priority",
            "speed_limit_topic",
            "odom_topic",
            "enable_stamped_cmd_vel",
            "use_sim_time",
            "costmap_update_timeout",
            "odom_duration",
            "publish_zero_velocity",
            "use_rotation_shim",
        ],
        "amcl" => &[
            "alpha1",
            "alpha2",
            "alpha3",
            "alpha4",
            "alpha5",
            "base_frame_id",
            "beam_skip_distance",
            "beam_skip_error_threshold",
            "beam_skip_threshold",
            "do_beamskip",
            "global_frame_id",
            "lambda_short",
            "laser_likelihood_max_dist",
            "laser_max_range",
            "laser_min_range",
            "laser_model_type",
            "set_initial_pose",
            "initial_pose",
            "max_beams",
            "max_particles",
            "min_particles",
            "odom_frame_id",
            "pf_err",
            "pf_z",
            "recovery_alpha_fast",
            "recovery_alpha_slow",
            "resample_interval",
            "robot_model_type",
            "save_pose_rate",
            "sigma_hit",
            "tf_broadcast",
            "transform_tolerance",
            "update_min_a",
            "update_min_d",
            "z_hit",
            "z_max",
            "z_rand",
            "z_short",
            "scan_topic",
            "map_topic",
            "first_map_only",
            "always_reset_initial_pose",
            "use_sim_time",
            "introspection_mode",
        ],
        "nav2_regulated_pure_pursuit_controller::RegulatedPurePursuitController" => &[
            "plugin",
            "desired_linear_vel",
            "lookahead_dist",
            "min_lookahead_dist",
            "max_lookahead_dist",
            "lookahead_time",
            "rotate_to_heading_angular_vel",
            "transform_tolerance",
            "use_velocity_scaled_lookahead_dist",
            "min_approach_linear_velocity",
            "approach_velocity_scaling_dist",
            "use_collision_detection",
            "max_allowed_time_to_collision_up_to_carrot",
            "use_regulated_linear_velocity_scaling",
            "use_cost_regulated_linear_velocity_scaling",
            "regulated_linear_scaling_min_radius",
            "regulated_linear_scaling_min_speed",
            "use_fixed_curvature_lookahead",
            "curvature_lookahead_dist",
            "use_rotate_to_heading",
            "allow_reversing",
            "rotate_to_heading_min_angle",
            "max_angular_accel",
            "max_robot_pose_search_dist",
            "cost_scaling_dist",
            "cost_scaling_gain",
            "inflation_cost_scaling_factor",
            "interpolate_curvature_after_goal",
            "stateful",
            "use_cancel_deceleration",
            "cancel_deceleration",
        ],
        _ => &[],
    }
}

/// Frame-valued Nav2 parameters.
pub const NAV2_FRAME_PARAMS: &[&str] = &[
    "global_frame",
    "robot_base_frame",
    "base_frame_id",
    "odom_frame_id",
    "global_frame_id",
    "base_frame",
    "odom_frame",
    "map_frame",
];

/// Frames that never come from the robot description.
pub const WORLD_FRAMES: &[&str] = &["map", "odom", "earth", "world", "utm"];

/// Packages that run a simulator (and publish /clock).
pub const SIM_PACKAGES: &[&str] = &[
    "gazebo_ros",
    "ros_gz_sim",
    "ros_gz_bridge",
    "ros_ign_gazebo",
    "ros_ign_bridge",
    "webots_ros2_driver",
    "gazebo_ros2_control",
    "gz_ros2_control",
    "isaac_ros_launch",
    "mvsim",
    "stage_ros2",
    "flatland_server",
];

/// Endpoints published by well-known nodes that are normally installed, not in the workspace.
/// (package, executable, topic as written, latched?)
pub fn known_publishers(pkg: &str, exec: &str) -> Vec<(&'static str, QosSpec)> {
    let latched = |o: &str| QosSpec {
        durability: Some(Durability::TransientLocal),
        reliability: Some(Reliability::Reliable),
        depth: Some(1),
        ..QosSpec::ros_default(o.to_string())
    };
    let normal = |o: &str| QosSpec::ros_default(o.to_string());
    match (pkg, exec) {
        ("robot_state_publisher", _) => vec![
            (
                "robot_description",
                latched("robot_state_publisher (transient local)"),
            ),
            (
                "/tf_static",
                latched("robot_state_publisher (transient local)"),
            ),
        ],
        ("nav2_map_server", "map_server") => {
            vec![("map", latched("nav2 map_server (transient local)"))]
        }
        ("joint_state_publisher" | "joint_state_publisher_gui", _) => {
            vec![("joint_states", normal("joint_state_publisher"))]
        }
        ("tf2_ros", "static_transform_publisher") => vec![(
            "/tf_static",
            latched("static_transform_publisher (transient local)"),
        )],
        _ => Vec::new(),
    }
}

/// Topics conventionally published once and latched.
pub const LATCHED_TOPICS: &[&str] = &[
    "map",
    "robot_description",
    "tf_static",
    "speed_limit",
    "costmap_filter_info",
];
