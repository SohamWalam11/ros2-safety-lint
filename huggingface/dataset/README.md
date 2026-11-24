---
license: apache-2.0
language:
  - en
pretty_name: rosfix knowledge packs
tags:
  - ros2
  - robotics
  - static-analysis
  - configuration
  - nav2
  - ros2-control
size_categories:
  - n<1K
---

# rosfix knowledge packs

What popular ROS 2 packages declare, per ROS distribution, extracted from their source by
[rosfix](https://github.com/SohamWalam11/ros2-safety-lint). rosfix embeds these files to check
configurations of packages you install from binaries (no source on your machine). Licensed under
Apache-2.0 or MIT, at your option. The data describes the listed upstream projects, which keep
their own licenses.

## Files

| File | Content |
|---|---|
| `packs/{humble,jazzy,kilted}.json.gz` | Per package: executables and rclcpp components, each node's declared parameters (name, type, default present), topics/services/actions with QoS; plus all pluginlib plugin class names and BehaviorTree.CPP node IDs registered by the packages |
| `keys/{humble,jazzy,kilted}.txt.gz` | Names `rosdep install` can resolve: packages released in the distribution (rosdistro) and rosdep system keys |
| `scripts/` | The scripts that rebuild both (`build_packs.sh`, `build_rosdep_keys.py`) |

Sources (each built from the distribution's own branch): ros-navigation/navigation2,
ros-controls/ros2_controllers, ros-controls/ros2_control, ros/robot_state_publisher,
ros/joint_state_publisher, ros2/geometry2, ros2/demos, SteveMacenski/slam_toolbox,
cra-ros-pkg/robot_localization. Each pack records the exact commit of every source in its
`sources` field. Test sources are excluded.

## Contents (rebuilt 24 November 2026)

| Distribution | Packages | Executables | Components | Parameter declarations | Topic endpoints | System-default QoS | pluginlib classes | BT node IDs | rosdep keys |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Humble | 42 | 100 | 54 | 2,335 | 290 | 4 | 242 | 54 | 4,590 |
| Jazzy | 44 | 96 | 38 | 3,051 | 240 | 6 | 281 | 65 | 4,696 |
| Kilted | 47 | 97 | 40 | 2,985 | 232 | 8 | 276 | 70 | 3,990 |

An endpoint created with `rclcpp::SystemDefaultsQoS()` or `rmw_qos_profile_system_default` has
`"system_default": true` and no reliability or durability. Its QoS is chosen by the middleware
(or a DDS profile file), so it is unknown, not the ROS default profile.

Changes in this rebuild: packs built before 24 November 2026 stored system-default endpoints
without the flag, and missed every parameter of Nav2's AMCL (declared through
`nav2_util::LifecycleNode::add_parameter`). AMCL now has 45 (Humble), 53 (Jazzy) and 54 (Kilted)
declared parameters.

A parameter name is `{"Exact": "name"}`, or `{"Suffix": "name"}` for a parameter declared under a
prefix only known at runtime (for example a plugin's own name).

## Uses

- Checking parameter names and types against what a node really declares (rosfix rules P003,
  P004, P007).
- Detecting renames between ROS releases: parameters (P007), plugin classes (N008),
  behavior-tree nodes (N009). One example: Nav2's `progress_checker_plugin` (Humble) became
  `progress_checker_plugins` (Jazzy).
- Validating `package.xml` dependencies against rosdep (L009).

## Loading

```python
import gzip, json
pack = json.load(gzip.open("packs/jazzy.json.gz"))
params = pack["packages"]["nav2_controller"]["executables"]["controller_server"]["params"]
print([next(iter(p["name"].values())) for p in params][:10])
keys = gzip.open("keys/jazzy.txt.gz").read().decode().split()
```

The format is versioned (`"format": 1`); fields added later are optional.
