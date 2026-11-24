# NoDL node interfaces

NoDL (Node Definition Language, ros-tooling/nodl, version 2) describes the public interface
of a ROS 2 node in YAML: its parameters with types, defaults and validation, and its
publishers, subscriptions, services and actions with QoS. rosfix reads NoDL documents and
writes them.

## Reading: installed packages

A package that registers a NoDL document with `ament_nodl_register(<name> FILE <file>)`
installs it into the ament index as `share/ament_index/resource_index/nodl/<package>__<name>`.
When the document is named after one of the package's executables and the package is not in
the workspace, rosfix uses it like a knowledge-pack entry for that executable: parameters set
in launch and parameter files are checked against the declared ones (P003 for undeclared
names, P004 for wrong types), and the declared endpoints and QoS take part in the topic and
QoS checks.

Rules:

    A knowledge-pack entry for the same executable wins over the NoDL document.
    A document with `include:` references marks the parameters as open, so nothing is
    reported as undeclared that an included interface might declare.
    Message types are read as `package/msg/Type`.

The installed environment comes from AMENT_PREFIX_PATH or `--ros-prefix`, as for everything
else (see `rosfix docs environment`).

## Writing: rosfix nodl export

    rosfix nodl export [PATH]              print one document per executable
    rosfix nodl export [PATH] -o nodl/     write nodl/<package>/<executable>.nodl.yaml

For every workspace executable whose source rosfix can link, export writes what it read in
the code: declared parameters with their types and literal defaults, and endpoints with
their message types and QoS. It is a starting point for adopting NoDL in an existing package,
not a replacement for writing the interface down on purpose.

Nothing is invented. When rosfix cannot determine something statically, the document says
so in a comment that starts with `# ?` instead of writing a guess:

    a default value that is computed rather than a literal (omitting it would make the
    parameter required, which would be wrong)
    a parameter whose type is unknown
    a QoS profile that is not fully known (a variable, an unknown profile class)
    an endpoint whose name is computed, or whose message type is not a full
    package/msg/Type name (Python nodes often record only the class name)
    parameters under a runtime prefix (plugins)

All 132 documents exported from nine of the evaluation repositories (demos, navigation2,
moveit2, ros2_control, turtlebot3, rmf_ros2, system_tests, sros2, rmw_fastrtps) validate
against the NoDL v2 JSON schema from ros-tooling/nodl.
