# Installed ROS and knowledge packs

Most workspaces launch nodes they do not contain: Nav2, ros2_controllers,
robot_state_publisher and so on. To check what is passed to those nodes, rosfix needs to
know what they declare. It has two sources.

## The installed environment

In a sourced ROS 2 shell, rosfix reads every prefix in AMENT_PREFIX_PATH through the ament
resource index:

    share/ament_index/resource_index/packages/<pkg>    which packages exist
    lib/<pkg>/                                         their executables
    ..._pluginlib__plugin/<pkg>                        exported pluginlib plugins
    rclcpp_components/<pkg>                            exported components
    share/<pkg>/                                       launch and config files

Launch and parameter files of installed packages are read only when a workspace launch
file includes them. Binary installs have no source code, so the installed environment
tells rosfix which executables and plugins exist, but not which parameters they declare.

`--ros-prefix DIR` reads another install instead (repeat it for several prefixes).
`--no-ros` ignores the installed environment entirely; results then depend only on the
workspace and the knowledge pack.

## Knowledge packs

A knowledge pack is a compressed list of the parameters (name, type, whether a default
exists) and endpoints (name, type, QoS) of popular packages, extracted once from their
source with rosfix's own extractors. rosfix embeds one pack per distribution:

    humble    40 packages, 152 executables and components
    jazzy     44 packages, 134 executables and components
    kilted    44 packages, 134 executables and components
    rolling   uses the kilted pack

The packs are built from navigation2, ros2_controllers, ros2_control,
robot_state_publisher, joint_state_publisher, geometry2, demos, slam_toolbox and
robot_localization. With a pack, rosfix reports, for example,
`controller_frequency: 20` for Nav2's controller server (an integer where a double is
declared, which crashes the node at startup) without having Nav2's source.

Parameter names differ between distributions, so a pack is used only when the
distribution is known: `--distro`, then `distro` in rosfix.toml, then ROS_DISTRO. Without
a distribution, nodes from installed packages are treated as unknown and their parameters
are not checked.

## Which source wins

A launched node is linked to source code in the workspace when its package is there. Only
when the package is not in the workspace is the knowledge pack consulted. A package
checked out into the workspace therefore overrides the pack, which is what you want when
you patch an upstream package locally. The installed environment never supplies
parameter declarations; it tells rosfix which packages, executables, plugins and
components exist, and where their launch and config files are.

## Rebuilding packs

The rosfix-pack tool in crates/ros2-safety-mutate builds a pack from source checkouts;
scripts/build_packs.sh builds all three. A pack has a format number; rosfix ignores packs
with a format it does not understand.
