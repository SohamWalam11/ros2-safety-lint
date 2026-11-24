# rosfix

Static checker for ROS 2 misconfigurations that fail silently: parameters that never reach
their node, topics that never connect, QoS pairs that never deliver, callbacks that
deadlock, controllers that do not match the hardware, launch files that start executables
which are not installed, and more.

```bash
cargo install rosfix
rosfix ~/ros2_ws/src
rosfix --format sarif -o rosfix.sarif
rosfix --fix            # preview verified fixes
rosfix --fix --write    # apply them
```

See the [project README](https://github.com/SohamWalam11/ros2-safety-lint#readme) for the
rule list, configuration, CI integration and evaluation.
