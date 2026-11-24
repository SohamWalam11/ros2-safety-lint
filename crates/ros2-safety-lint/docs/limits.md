# Limits

rosfix reads files. It does not build, launch or observe a running system, so some
problems are out of its reach by design.

Values computed at runtime. Node names, topic names and parameter values built from
environment variables, command line arguments or arbitrary Python are unknown. Checks
that depend on them stay silent. `rosfix graph` marks launch files with such parts as
partial, and nodes whose parameter sources could not all be read with "+?".

Parameters declared by plugins. A node that loads plugins (Nav2 servers, controller
managers) accepts parameters that only the plugin declares. rosfix checks the top-level
parameters of such nodes and leaves dotted plugin parameters alone unless the plugin is in
the workspace or a knowledge pack.

Packages outside the workspace and the packs. Installed packages without source tell
rosfix which executables and plugins exist, but not which parameters they declare. Nodes
of such packages are treated as external and their parameters are not checked.

Behaviour, not configuration. rosfix finds configuration that is inconsistent or ignored.
It does not judge whether a consistent configuration is a good one: a valid but
unsuitable controller gain, an inflation radius that is legal but too small for the
corridor, a timeout that is too short for the hardware.

Middleware and network. QoS compatibility is checked from the QoS declared in code. A
profile left to the middleware (`SystemDefaultsQoS()`, `rmw_qos_profile_system_default`) is
unknown, because the DDS implementation picks those policies; it is resolved only when the
launch file points Fast DDS at a profile file in the workspace
(`FASTRTPS_DEFAULT_PROFILES_FILE` or `FASTDDS_DEFAULT_PROFILES_FILE`), from the profile named
after the topic or the default profile. Discovery, firewalls, multicast and other DDS vendor
settings beyond the checks of the network agent and `rosfix doctor` are not examined.

What a clean result means. No findings means rosfix saw nothing wrong in what it could
resolve. It is not a proof of correctness. In a study of real configuration fix commits
across the ROS 2 ecosystem (research/ in the repository), most fixes concerned packaging
and build details that rosfix does not model, and its recall on all genuine fixes was
low. The precision of what it does report is the property it is built for.
