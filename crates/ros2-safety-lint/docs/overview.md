# Overview

rosfix is a static checker for ROS 2 workspaces. It looks for configuration mistakes that
ROS 2 does not report: a parameter under the wrong node key is dropped, a remap that
matches nothing does nothing, a reliable subscriber never hears a best-effort publisher,
a lifecycle node that no manager lists never leaves the unconfigured state. None of these
print an error. The robot simply behaves differently from what the files say.

rosfix reads the workspace as a whole, works out what would be launched, and checks the
pieces against each other before anything runs. It needs no ROS installation, no build,
and no running system. It runs on one thread by default and typically uses 6 to 70 MB of
memory.

## What it reads

Launch files (Python, XML and YAML), parameter YAML, C++ and Python node sources,
package.xml, CMakeLists.txt, setup.py and setup.cfg, plugin description XML,
generate_parameter_library specs, URDF and xacro, ros2_control and Nav2 configuration,
RViz configuration, MoveIt configuration (SRDF, kinematics.yaml, joint_limits.yaml,
moveit_controllers.yaml), ros_gz_bridge configuration and parameter_bridge arguments,
and SROS2 governance, permissions and policy files.

Python launch files are never executed. A small interpreter evaluates the parts that can
be known statically (literals, launch arguments with defaults, path joins, package share
directories, conditions with known values). Anything it cannot evaluate is marked as
unknown.

## How one analysis works

1. Index. Every relevant file is parsed once into small typed records called facts:
   packages and what they build and install, nodes with their declared parameters,
   publishers, subscriptions, QoS and callbacks, parameter file sections, URDF links,
   joints and ros2_control blocks, security policies, and files that could not be parsed.

2. Resolve. Launch files are evaluated the way ROS 2 would evaluate them: namespaces,
   remaps, launch arguments, includes, conditions and parameter files. The result is a
   launch graph of node instances with fully qualified names and bound parameters. Each
   instance is linked to the source code that implements it, either in the workspace, in
   the installed ROS environment, or in a knowledge pack.

3. Agents. Each problem area has an agent (parameters, topics, QoS, executors,
   ros2_control, TF, lifecycle, launch arguments, middleware environment, packaging,
   simulated time, Nav2, MoveIt, ros_gz_bridge, robot description physics, ROS 1
   leftovers, SROS2). An agent runs only if the workspace contains something it checks.
   Agents read the index and graph but never change them, so the order in which they run
   does not affect the results.

4. Post-process. Rule settings from rosfix.toml, suppression comments and baselines are
   applied, and the report is written.

## When rosfix cannot know, it stays silent

Every check depends on facts. When a fact cannot be determined statically, for example a
node name computed at runtime, parameters declared by a plugin, or a launch file that
calls arbitrary Python, rosfix records it as unknown and turns off the checks that depend
on it. A missing finding therefore means "nothing wrong that rosfix could see", not "this
is correct". `rosfix graph` shows what was understood and what was left open.

## Where to go next

    rosfix docs install      installing rosfix
    rosfix docs commands     every command and option
    rosfix docs config       rosfix.toml
    rosfix docs fixes        how automatic fixes are verified
    rosfix docs rules        every rule, with an explanation
    rosfix docs limits       what static analysis cannot see
