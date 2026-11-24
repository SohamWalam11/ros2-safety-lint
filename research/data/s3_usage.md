# Study 3: removed Humble parameters still set on Jazzy branches

Parameters checked (source-verified removals/renames with a node marker): 10.
Repositories scanned: 830. Hits: 199 lines in 118 files of 33 repositories.

| Repository | File:line | Node | Parameter | Change |
|---|---|---|---|---|
| blackcoffeerobotics/bcr_bot | `config/nav2_params.yaml:312` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| brukg/SO-100-arm | `so_arm_100_moveit_config/config/controllers_5dof.yaml:28` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| brukg/SO-100-arm | `so_arm_100_moveit_config/config/controllers_7dof.yaml:28` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| brukg/SO-100-arm | `so_arm_100_moveit_config/config/hardware_controllers.yaml:36` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| brukg/SO-100-arm | `so_arm_100_moveit_config/config/hardware_controllers.yaml:72` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| clearpathrobotics/clearpath_common | `clearpath_manipulators_description/config/lift/ewellix/control_jtc.yaml:20` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| clearpathrobotics/clearpath_common | `clearpath_manipulators_description/config/arm/kinova_gen3_6dof/control.yaml:26` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| clearpathrobotics/clearpath_common | `clearpath_manipulators_description/config/arm/kinova_gen3_7dof/control.yaml:27` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| clearpathrobotics/clearpath_common | `clearpath_manipulators_description/config/arm/kinova_gen3_lite/control.yaml:25` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| clearpathrobotics/clearpath_common | `clearpath_manipulators_description/config/arm/universal_robots/control.yaml:25` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/w200/control/diff_4wd.yaml:35` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/w200/control/diff_4wd.yaml:17` | diff_drive_controller/diff_drive_controller | `wheels_per_side` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/r100/control/diff_4wd.yaml:35` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/r100/control/diff_4wd.yaml:17` | diff_drive_controller/diff_drive_controller | `wheels_per_side` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/r100/control/omni_4wd.yaml:30` | mecanum_drive_controller/mecanum_drive_controller | `use_stamped_vel` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/j100/control/diff_4wd.yaml:35` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/j100/control/diff_4wd.yaml:17` | diff_drive_controller/diff_drive_controller | `wheels_per_side` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/do150/control/diff_4wd.yaml:35` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/do150/control/diff_4wd.yaml:17` | diff_drive_controller/diff_drive_controller | `wheels_per_side` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/do150/control/omni_4wd.yaml:30` | mecanum_drive_controller/mecanum_drive_controller | `use_stamped_vel` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/do100/control/diff_4wd.yaml:35` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/do100/control/diff_4wd.yaml:17` | diff_drive_controller/diff_drive_controller | `wheels_per_side` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/do100/control/omni_4wd.yaml:30` | mecanum_drive_controller/mecanum_drive_controller | `use_stamped_vel` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/dd150/control/diff_fwd.yaml:35` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/dd150/control/diff_fwd.yaml:17` | diff_drive_controller/diff_drive_controller | `wheels_per_side` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/dd100/control/diff_fwd.yaml:35` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/dd100/control/diff_fwd.yaml:17` | diff_drive_controller/diff_drive_controller | `wheels_per_side` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/a300/control/diff_4wd.yaml:17` | diff_drive_controller/diff_drive_controller | `wheels_per_side` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/a300/control/diff_fwd.yaml:17` | diff_drive_controller/diff_drive_controller | `wheels_per_side` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/a300/control/diff_rwd.yaml:17` | diff_drive_controller/diff_drive_controller | `wheels_per_side` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/a300/control/omni_4wd.yaml:30` | mecanum_drive_controller/mecanum_drive_controller | `use_stamped_vel` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/a200/control/diff_4wd.yaml:35` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| clearpathrobotics/clearpath_common | `clearpath_control/config/a200/control/diff_4wd.yaml:17` | diff_drive_controller/diff_drive_controller | `wheels_per_side` | removed |
| clearpathrobotics/clearpath_nav2_demos | `config/a200/nav2.yaml:320` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| clearpathrobotics/clearpath_nav2_demos | `config/a300/nav2.yaml:319` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| clearpathrobotics/clearpath_nav2_demos | `config/dd100/nav2.yaml:319` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| clearpathrobotics/clearpath_nav2_demos | `config/dd150/nav2.yaml:319` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| clearpathrobotics/clearpath_nav2_demos | `config/do100/nav2.yaml:324` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| clearpathrobotics/clearpath_nav2_demos | `config/do150/nav2.yaml:324` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| clearpathrobotics/clearpath_nav2_demos | `config/j100/nav2.yaml:319` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| clearpathrobotics/clearpath_nav2_demos | `config/r100/nav2.yaml:324` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| clearpathrobotics/clearpath_nav2_demos | `config/w200/nav2.yaml:319` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| clearpathrobotics/ewellix_lift_common | `ewellix_description/config/control/jtc.yaml:20` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| cps-test-lab/scenario-execution | `simulation/gazebo/tb4_sim_scenario/params/nav2_params.yaml:309` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| cps-test-lab/scenario-execution | `simulation/gazebo/tb4_sim_scenario/params/nav2_params.yaml:310` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| cps-test-lab/scenario-execution | `simulation/gazebo/tb4_sim_scenario/params/nav2_params.yaml:122` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| cyberbotics/webots_ros2 | `webots_ros2_turtlebot/resource/nav2_params.yaml:112` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| cyberbotics/webots_ros2 | `webots_ros2_turtlebot/resource/ros2control.yml:22` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| cyberbotics/webots_ros2 | `webots_ros2_tiago/resource/nav2_params.yaml:302` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| cyberbotics/webots_ros2 | `webots_ros2_tiago/resource/nav2_params.yaml:303` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| cyberbotics/webots_ros2 | `webots_ros2_tiago/resource/nav2_params.yaml:123` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| cyberbotics/webots_ros2 | `webots_ros2_tiago/resource/nav2_params_iron.yaml:306` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| cyberbotics/webots_ros2 | `webots_ros2_tiago/resource/nav2_params_iron.yaml:307` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| cyberbotics/webots_ros2 | `webots_ros2_tiago/resource/nav2_params_iron.yaml:127` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| cyberbotics/webots_ros2 | `webots_ros2_tiago/resource/ros2_control.yml:22` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| cyberbotics/webots_ros2 | `webots_ros2_husarion/resource/rosbot_controllers.yaml:22` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| cyberbotics/webots_ros2 | `webots_ros2_husarion/resource/rosbot_xl_controllers.yaml:22` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| cyberbotics/webots_ros2 | `webots_ros2_epuck/resource/nav2_params.yaml:302` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| cyberbotics/webots_ros2 | `webots_ros2_epuck/resource/nav2_params.yaml:303` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| cyberbotics/webots_ros2 | `webots_ros2_epuck/resource/nav2_params.yaml:123` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| cyberbotics/webots_ros2 | `webots_ros2_epuck/resource/nav2_params_iron.yaml:306` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| cyberbotics/webots_ros2 | `webots_ros2_epuck/resource/nav2_params_iron.yaml:307` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| cyberbotics/webots_ros2 | `webots_ros2_epuck/resource/nav2_params_iron.yaml:127` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| cyberbotics/webots_ros2 | `webots_ros2_epuck/resource/ros2_control.yml:22` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| EasyNavigation/easynav_playgrounds | `playground_tiago/easynav_playground_tiago_description/config/tiago_controllers.yaml:42` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| EasyNavigation/easynav_playgrounds | `playground_summit/easynav_playground_summit_description/config/summit_controllers.yaml:34` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| EliteRobots/Elite_Robots_CS_ROS2_Driver | `elite_robots_driver/config/elite_cs_controllers.yaml:70` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| EliteRobots/Elite_Robots_CS_ROS2_Driver | `elite_robots_driver/config/elite_cs_controllers.yaml:100` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| EliteRobots/Elite_Robots_CS_ROS2_Driver | `elite_robots_driver/config/elite_cs_controllers.yaml:117` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| EliteRobots/Elite_Robots_CS_ROS2_Driver | `elite_robots_driver/config/elite_cs_controllers_5f.yaml:68` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| EliteRobots/Elite_Robots_CS_ROS2_Driver | `elite_robots_driver/config/elite_cs_controllers_5f.yaml:96` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| EliteRobots/Elite_Robots_CS_ROS2_Driver | `elite_robots_driver/config/elite_cs_controllers_5f.yaml:112` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| husarion/husarion_components_description | `config/kinova_6dof_controllers.yaml:18` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| husarion/husarion_components_description | `config/kinova_7dof_controllers.yaml:19` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| husarion/husarion_components_description | `config/ur_controllers.yaml:18` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| IMRCLab/crazyswarm2 | `crazyflie_examples/config/nav2_params.yaml:9` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| introlab/rtabmap_ros | `rtabmap_demos/params/champ_nav2_params.yaml:237` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/champ_nav2_params.yaml:238` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/champ_nav2_params.yaml:80` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| introlab/rtabmap_ros | `rtabmap_demos/params/isaac_nav2_params.yaml:244` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/isaac_nav2_params.yaml:245` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/isaac_nav2_params.yaml:80` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| introlab/rtabmap_ros | `rtabmap_demos/params/isaac_vslam_nav2_params.yaml:244` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/isaac_vslam_nav2_params.yaml:245` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/isaac_vslam_nav2_params.yaml:80` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| introlab/rtabmap_ros | `rtabmap_demos/params/turtlebot3_fusioncore_icp_nav2_params.yaml:275` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/turtlebot3_fusioncore_icp_nav2_params.yaml:221` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/turtlebot3_fusioncore_icp_nav2_params.yaml:276` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/turtlebot3_nav2_params.yaml:402` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/turtlebot3_nav2_params.yaml:341` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/turtlebot3_nav2_params.yaml:403` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/turtlebot3_rgbd_nav2_params.yaml:399` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/turtlebot3_rgbd_nav2_params.yaml:338` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/turtlebot3_rgbd_nav2_params.yaml:400` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/turtlebot3_rgbd_scan_nav2_params.yaml:409` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/turtlebot3_rgbd_scan_nav2_params.yaml:348` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/turtlebot3_rgbd_scan_nav2_params.yaml:410` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/turtlebot3_scan_nav2_params.yaml:403` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/turtlebot3_scan_nav2_params.yaml:342` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/turtlebot3_scan_nav2_params.yaml:404` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/humble/turtlebot3_rgbd_nav2_params.yaml:236` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/humble/turtlebot3_rgbd_nav2_params.yaml:237` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/humble/turtlebot3_rgbd_nav2_params.yaml:80` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| introlab/rtabmap_ros | `rtabmap_demos/params/humble/turtlebot3_rgbd_scan_nav2_params.yaml:250` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/humble/turtlebot3_rgbd_scan_nav2_params.yaml:251` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/humble/turtlebot3_rgbd_scan_nav2_params.yaml:80` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| introlab/rtabmap_ros | `rtabmap_demos/params/humble/turtlebot3_scan_nav2_params.yaml:244` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/humble/turtlebot3_scan_nav2_params.yaml:245` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| introlab/rtabmap_ros | `rtabmap_demos/params/humble/turtlebot3_scan_nav2_params.yaml:80` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| iRobotEducation/create3_sim | `irobot_create_common/irobot_create_control/config/control.yaml:40` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| Kinovarobotics/ros2_kortex | `kortex_moveit_config/kinova_gen3_lite_moveit_config/config/ros2_controllers.yaml:34` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| Kinovarobotics/ros2_kortex | `kortex_moveit_config/kinova_gen3_7dof_robotiq_2f_85_moveit_config/config/ros2_controllers.yaml:35` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| Kinovarobotics/ros2_kortex | `kortex_moveit_config/kinova_gen3_6dof_robotiq_2f_85_moveit_config/config/ros2_controllers.yaml:34` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| Kinovarobotics/ros2_kortex | `kortex_description/arms/gen3_lite/6dof/config/ros2_controllers.yaml:34` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| Kinovarobotics/ros2_kortex | `kortex_description/arms/gen3/7dof/config/ros2_controllers.yaml:35` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| Kinovarobotics/ros2_kortex | `kortex_description/arms/gen3/6dof/config/ros2_controllers.yaml:34` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| kuka-ros/kuka_robot_descriptions | `kuka_resources/config/fake_hardware_config_6_axis.yaml:26` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| kuka-ros/kuka_robot_descriptions | `kuka_resources/config/fake_hardware_config_6_axis_kl.yaml:27` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| kuka-ros/kuka_robot_descriptions | `kuka_resources/config/fake_hardware_config_7_axis.yaml:27` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| manankharwar/fusioncore | `fusioncore_ros/config/nav2_params.yaml:189` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| manankharwar/fusioncore | `fusioncore_ros/config/nav2_params.yaml:190` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| manankharwar/fusioncore | `fusioncore_ros/config/nav2_params.yaml:63` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| MRPT/mvsim | `mvsim_tutorial/ros2_control/agricobiot2_controllers.yaml:48` | steering_controllers_library/steering_controllers_library | `use_stamped_vel` | removed |
| MRPT/mvsim | `mvsim_tutorial/ros2_control/car_ackermann_controllers.yaml:51` | steering_controllers_library/steering_controllers_library | `use_stamped_vel` | removed |
| neobotix/neo_nav2_bringup | `config/navigation.yaml:306` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| neobotix/neo_nav2_bringup | `config/navigation.yaml:307` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| neobotix/neo_nav2_bringup | `config/navigation.yaml:148` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| robosoft-ai/SMACC2 | `smacc2_sm_reference_library/sm_nav2_gazebo_test_3/config/nav2_params.yaml:525` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| robosoft-ai/SMACC2 | `smacc2_sm_reference_library/sm_nav2_gazebo_test_3/config/nav2_params.yaml:477` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| robosoft-ai/SMACC2 | `smacc2_sm_reference_library/sm_nav2_gazebo_test_3/config/nav2_params.yaml:526` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| robosoft-ai/SMACC2 | `smacc2_sm_reference_library/sm_nav2_gazebo_test_2/config/nav2_params.yaml:523` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| robosoft-ai/SMACC2 | `smacc2_sm_reference_library/sm_nav2_gazebo_test_2/config/nav2_params.yaml:475` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| robosoft-ai/SMACC2 | `smacc2_sm_reference_library/sm_nav2_gazebo_test_2/config/nav2_params.yaml:524` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ROBOTIS-GIT/turtlebot3 | `turtlebot3_navigation2/param/burger.yaml:373` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ROBOTIS-GIT/turtlebot3 | `turtlebot3_navigation2/param/burger_cam.yaml:373` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ROBOTIS-GIT/turtlebot3 | `turtlebot3_navigation2/param/waffle.yaml:373` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ROBOTIS-GIT/turtlebot3 | `turtlebot3_navigation2/param/waffle_pi.yaml:373` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ROBOTIS-GIT/turtlebot3 | `turtlebot3_navigation2/param/humble/burger.yaml:107` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| ROBOTIS-GIT/turtlebot3 | `turtlebot3_navigation2/param/humble/burger_cam.yaml:107` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| ROBOTIS-GIT/turtlebot3 | `turtlebot3_navigation2/param/humble/waffle.yaml:107` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| ROBOTIS-GIT/turtlebot3 | `turtlebot3_navigation2/param/humble/waffle_pi.yaml:106` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| ROBOTIS-GIT/turtlebot3_home_service_challenge | `turtlebot3_home_service_challenge_tools/param/turtlebot3_hsc.yaml:295` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| ROBOTIS-GIT/turtlebot3_home_service_challenge | `turtlebot3_home_service_challenge_tools/param/turtlebot3_hsc.yaml:296` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ROBOTIS-GIT/turtlebot3_home_service_challenge | `turtlebot3_home_service_challenge_tools/param/turtlebot3_hsc.yaml:116` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| ROBOTIS-GIT/turtlebot3_manipulation | `turtlebot3_manipulation_navigation2/param/turtlebot3.yaml:295` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| ROBOTIS-GIT/turtlebot3_manipulation | `turtlebot3_manipulation_navigation2/param/turtlebot3.yaml:296` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ROBOTIS-GIT/turtlebot3_manipulation | `turtlebot3_manipulation_navigation2/param/turtlebot3.yaml:116` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| ROBOTIS-GIT/turtlebot3_manipulation | `turtlebot3_manipulation_navigation2/param/turtlebot3_use_sim_time.yaml:295` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| ROBOTIS-GIT/turtlebot3_manipulation | `turtlebot3_manipulation_navigation2/param/turtlebot3_use_sim_time.yaml:296` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ROBOTIS-GIT/turtlebot3_manipulation | `turtlebot3_manipulation_navigation2/param/turtlebot3_use_sim_time.yaml:116` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| ROBOTIS-GIT/turtlebot3_manipulation | `turtlebot3_manipulation_bringup/config/gazebo_controller_manager.yaml:41` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| ROBOTIS-GIT/turtlebot3_manipulation | `turtlebot3_manipulation_bringup/config/hardware_controller_manager.yaml:44` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| ROBOTIS-GIT/turtlebot3_manipulation | `turtlebot3_manipulation_bringup/config/hardware_controller_manager.yaml:98` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| ros-controls/gz_ros2_control | `gz_ros2_control_demos/config/diff_drive_controller.yaml:33` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| ros-controls/topic_based_hardware_interfaces | `joint_state_topic_hardware_interface/test/rrr/ros2_controllers.yaml:21` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| ros-industrial/ros2_canopen | `canopen_tests/config/cia402_multichannel_system/ros2_controllers.yaml:47` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| ros-industrial/ros2_canopen | `canopen_tests/config/cia402_namespaced_system/ros2_controllers.yaml:36` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| ros-industrial/ros2_canopen | `canopen_tests/config/cia402_system/ros2_controllers.yaml:36` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| ros-planning/navigation2 | `nav2_system_tests/src/costmap_filters/keepout_params.yaml:226` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| ros-planning/navigation2 | `nav2_system_tests/src/costmap_filters/keepout_params.yaml:227` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ros-planning/navigation2 | `nav2_system_tests/src/costmap_filters/keepout_params.yaml:287` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ros-planning/navigation2 | `nav2_system_tests/src/costmap_filters/speed_global_params.yaml:216` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| ros-planning/navigation2 | `nav2_system_tests/src/costmap_filters/speed_global_params.yaml:217` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ros-planning/navigation2 | `nav2_system_tests/src/costmap_filters/speed_global_params.yaml:277` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ros-planning/navigation2 | `nav2_system_tests/src/costmap_filters/speed_local_params.yaml:216` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| ros-planning/navigation2 | `nav2_system_tests/src/costmap_filters/speed_local_params.yaml:217` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ros-planning/navigation2 | `nav2_system_tests/src/costmap_filters/speed_local_params.yaml:277` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ros-planning/navigation2 | `nav2_system_tests/src/error_codes/error_code_param.yaml:8` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| ros-planning/navigation2 | `nav2_system_tests/src/gps_navigation/nav2_no_map_params.yaml:280` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ros-planning/navigation2 | `nav2_system_tests/src/system/nav2_system_params.yaml:309` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ros-planning/navigation2 | `nav2_bringup/params/nav2_multirobot_params_1.yaml:211` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| ros-planning/navigation2 | `nav2_bringup/params/nav2_multirobot_params_1.yaml:212` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ros-planning/navigation2 | `nav2_bringup/params/nav2_multirobot_params_1.yaml:259` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ros-planning/navigation2 | `nav2_bringup/params/nav2_multirobot_params_2.yaml:210` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| ros-planning/navigation2 | `nav2_bringup/params/nav2_multirobot_params_2.yaml:211` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ros-planning/navigation2 | `nav2_bringup/params/nav2_multirobot_params_2.yaml:258` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ros-planning/navigation2 | `nav2_bringup/params/nav2_multirobot_params_all.yaml:321` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ros-planning/navigation2 | `nav2_bringup/params/nav2_params.yaml:451` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| ros-planning/navigation2 | `nav2_bringup/params/nav2_params.yaml:391` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| ros-planning/navigation2 | `nav2_bringup/params/nav2_params.yaml:452` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| rt-net/raspimouse_sim | `raspimouse_gazebo/config/raspimouse_controllers.yaml:30` | diff_drive_controller/diff_drive_controller | `use_stamped_vel` | removed |
| rt-net/raspimouse_sim | `raspimouse_gazebo/config/raspimouse_controllers.yaml:20` | diff_drive_controller/diff_drive_controller | `wheels_per_side` | removed |
| rt-net/raspimouse_slam_navigation_ros2 | `raspimouse_navigation/params/raspimouse.yaml:263` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| rt-net/raspimouse_slam_navigation_ros2 | `raspimouse_navigation/params/raspimouse.yaml:264` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| shimz-robotics/mapoi | `mapoi_turtlebot3_example/param/humble/burger.yaml:112` | nav2_controller/controller_server | `progress_checker_plugin` | renamed to `progress_checker_plugins` |
| shimz-robotics/mapoi | `mapoi_turtlebot3_example/param/jazzy/burger.yaml:371` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| simone-contorno/prox_mpc | `prox_mpc_demo/config/nav2_prox_mpc.yaml:403` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| simone-contorno/prox_mpc | `prox_mpc_demo/config/nav2_prox_mpc.yaml:352` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| simone-contorno/prox_mpc | `prox_mpc_demo/config/nav2_prox_mpc.yaml:404` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| simone-contorno/prox_mpc | `prox_mpc_demo/config/nav2_prox_mpc_predictive.yaml:457` | nav2_behaviors/behavior_server | `costmap_topic` | renamed to `local_costmap_topic` |
| simone-contorno/prox_mpc | `prox_mpc_demo/config/nav2_prox_mpc_predictive.yaml:406` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| simone-contorno/prox_mpc | `prox_mpc_demo/config/nav2_prox_mpc_predictive.yaml:458` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| turtlebot/turtlebot4 | `turtlebot4_navigation/config/nav2.yaml:312` | nav2_behaviors/behavior_server | `footprint_topic` | renamed to `local_footprint_topic` |
| UniversalRobots/Universal_Robots_ROS2_Driver | `ur_robot_driver/config/ur_controllers.yaml:67` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| UniversalRobots/Universal_Robots_ROS2_Driver | `ur_robot_driver/config/ur_controllers.yaml:110` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| UniversalRobots/Universal_Robots_ROS2_Driver | `ur_robot_driver/config/ur_controllers.yaml:138` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| UniversalRobots/Universal_Robots_ROS2_GZ_Simulation | `ur_simulation_gz/config/ur_controllers.yaml:26` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| UniversalRobots/Universal_Robots_ROS2_GZ_Simulation | `ur_simulation_gz/config/ur_controllers.yaml:42` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |
| UniversalRobots/Universal_Robots_ROS2_GZ_Simulation | `ur_simulation_gz/doc/resources/ur_controllers_test.yaml:26` | joint_trajectory_controller/joint_trajectory_controller | `state_publish_rate` | removed |