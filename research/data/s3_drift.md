# Study 3: parameter drift between distributions

Raw = pack/definition diff. Removed and renamed names are then checked against upstream
source (see `verify`): only `real` ones are breaking changes.

| Source | Transition | Nodes | Params before | Removed+renamed (raw) | real | still present | test artifact | unknown | Retyped | Added |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| packs | humble->jazzy | 36 | 1311 | 90 | 3 | 16 | 68 | 3 | 1 | 479 |
| packs | jazzy->kilted | 54 | 2119 | 18 | 8 | 0 | 1 | 9 | 0 | 42 |
| ros2_controllers | humble->jazzy | 19 | 199 | 8 | 7 | 1 | 0 | 0 | 0 | 81 |
| ros2_controllers | jazzy->kilted | 25 | 347 | 27 | 27 | 0 | 0 | 0 | 0 | 3 |

## Breaking changes per node (removed / renamed / retyped)

### packs:humble->jazzy `nav2_behaviors/behavior_server`
- renamed `costmap_topic` -> `local_costmap_topic` (similarity 0.81)
- renamed `footprint_topic` -> `local_footprint_topic` (similarity 0.83)

### packs:humble->jazzy `nav2_collision_monitor/collision_monitor`
- type `*.points`: DoubleArray -> Str

### packs:humble->jazzy `nav2_controller/controller_server`
- renamed `progress_checker_plugin` -> `progress_checker_plugins` (similarity 0.98)

### packs:jazzy->kilted `nav2_behaviors/behavior_server`
- removed `action_server_result_timeout`

### packs:jazzy->kilted `nav2_controller/controller_server`
- removed `action_server_result_timeout`

### packs:jazzy->kilted `nav2_planner/planner_server`
- removed `action_server_result_timeout`

### packs:jazzy->kilted `nav2_route/route_server`
- removed `action_server_result_timeout`

### packs:jazzy->kilted `nav2_smoother/smoother_server`
- removed `action_server_result_timeout`

### packs:jazzy->kilted `nav2_velocity_smoother/velocity_smoother`
- removed `stamp_smoothed_velocity_with_smoothing_time`

### packs:jazzy->kilted `nav2_waypoint_follower/waypoint_follower`
- removed `action_server_result_timeout`

### packs:jazzy->kilted `opennav_docking/opennav_docking`
- removed `action_server_result_timeout`

### ros2_controllers:humble->jazzy `diff_drive_controller/diff_drive_controller`
- removed `use_stamped_vel`
- removed `wheels_per_side`

### ros2_controllers:humble->jazzy `joint_trajectory_controller/joint_trajectory_controller`
- removed `gains.<joints>.angle_wraparound`
- removed `gains.<joints>.normalize_error`
- removed `state_publish_rate`

### ros2_controllers:humble->jazzy `mecanum_drive_controller/mecanum_drive_controller`
- removed `use_stamped_vel`

### ros2_controllers:humble->jazzy `steering_controllers_library/steering_controllers_library`
- removed `use_stamped_vel`

### ros2_controllers:jazzy->kilted `ackermann_steering_controller/ackermann_steering_controller`
- removed `front_wheel_track`
- removed `front_wheels_radius`
- removed `rear_wheel_track`
- removed `rear_wheels_radius`

### ros2_controllers:jazzy->kilted `bicycle_steering_controller/bicycle_steering_controller`
- removed `front_wheel_radius`
- removed `rear_wheel_radius`

### ros2_controllers:jazzy->kilted `diff_drive_controller/diff_drive_controller`
- removed `angular.z.has_acceleration_limits`
- removed `angular.z.has_jerk_limits`
- removed `angular.z.has_velocity_limits`
- removed `angular.z.min_acceleration`
- removed `linear.x.has_acceleration_limits`
- removed `linear.x.has_jerk_limits`
- removed `linear.x.has_velocity_limits`
- removed `linear.x.min_acceleration`

### ros2_controllers:jazzy->kilted `joint_trajectory_controller/joint_trajectory_controller`
- removed `gains.<joints>.i_clamp`
- removed `open_loop_control`

### ros2_controllers:jazzy->kilted `pid_controller/pid_controller`
- removed `enable_feedforward`
- removed `gains.<dof_names>.antiwindup`

### ros2_controllers:jazzy->kilted `pose_broadcaster/pose_broadcaster`
- removed `tf.publish_rate`

### ros2_controllers:jazzy->kilted `steering_controllers_library/steering_controllers_library`
- removed `front_steering`
- removed `front_wheels_names`
- removed `front_wheels_state_names`
- removed `rear_wheels_names`
- removed `rear_wheels_state_names`

### ros2_controllers:jazzy->kilted `tricycle_steering_controller/tricycle_steering_controller`
- removed `front_wheels_radius`
- removed `rear_wheels_radius`
- removed `wheel_track`
