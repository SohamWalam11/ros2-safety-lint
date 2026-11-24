# rosfix triage

    rosfix triage LOG [PATH]
    ros2 launch my_bringup robot.launch.py 2>&1 | rosfix triage - src

rosfix normally finds problems before anything runs. triage works the other way round:
given the output of a failed `ros2 launch` or of a single node, it finds the error lines it
recognises and, for each one, the findings in the workspace that explain it.

LOG is a file, or `-` to read standard input. PATH is the workspace, analysed exactly as
`rosfix check` would analyse it, with the same `--distro`, `--ros-prefix`, `--no-ros`,
`--config` and `--agents` options. Several explanations depend on the knowledge pack, so
pass `--distro` when ROS_DISTRO is not set.

## Output

For each recognised line: its line number in the log, the log text, what the error means,
what it is about (a node, class, topic, parameter or package), the findings that explain it
with their file:line:column, and a hint when no finding matches. `--format json` prints
the same as a list of objects with the fields line, log, problem, subject, findings (rule,
location, message) and hint. `-o FILE` writes to a file.

triage always exits with code 0. A log line that no finding explains is still listed, with
the hint, so you know rosfix recognised it but could not trace it to the workspace.

## Recognised errors

    pluginlib: class X with base class type Y does not exist
    components: failed to find class with the requested plugin name
    lifecycle: failed to change state for node
    launch: launch configuration 'x' does not exist
    launch: executable 'x' not found, package 'x' not found
    rosdep: cannot locate rosdep definition for [x]
    QoS: incompatible QoS for a topic
    Nav2 / BehaviorTree.CPP: node not recognized
    ros2_control: controllers that fail to load or configure
    tf2: invalid frame IDs
    parameters: invalid type, or parameter not declared

Lines in other formats are ignored. The log text is read only locally and is never sent
anywhere.
