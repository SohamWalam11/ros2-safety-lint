# rosfix doctor

    rosfix doctor

Checks the ROS 2 environment of the shell it runs in. The workspace checks (the network
agent, rules D001 to D005) see what launch files put into a node's environment; doctor
sees what the shell itself sets, which is where most middleware problems come from: a
setup.bash sourced from a deleted workspace, a mistyped RMW_IMPLEMENTATION exported in
.bashrc, a Fast DDS profile copied from a container setup.

Each check prints one line: a level (ok, note, warning, error), the variable, and what was
found, followed by a hint when there is something to do. The exit code is 1 if any check
is an error, else 0. doctor depends on the machine, so it is never part of `rosfix check`.

## What it checks

ROS_DISTRO
    Warning when unset (no ROS 2 environment sourced). Note when it names a distribution
    without a knowledge pack.

AMENT_PREFIX_PATH
    Warning when unset, and for every prefix that does not exist. A missing prefix usually
    means a workspace was moved or deleted after it was sourced.

RMW_IMPLEMENTATION
    Error when the value is a near miss of a known middleware (rmw_cyclonedds_cp for
    rmw_cyclonedds_cpp), with the likely intended name. Error when it names a known
    middleware that is not installed in any prefix: every node would abort at startup.

ROS_DOMAIN_ID
    Error when it is not an integer from 0 to 232. Note for 102 to 214, which can collide
    with ephemeral ports on Linux.

ROS_LOCALHOST_ONLY
    Warning on Jazzy and newer, where it is deprecated in favour of
    ROS_AUTOMATIC_DISCOVERY_RANGE, with an extra hint when both are set. Warning for any
    value other than 0 or 1.

ROS_AUTOMATIC_DISCOVERY_RANGE
    Error for a value other than SUBNET, LOCALHOST, OFF or SYSTEM_DEFAULT. Note on Humble,
    which does not read it.

CYCLONEDDS_URI, FASTRTPS_DEFAULT_PROFILES_FILE, FASTDDS_DEFAULT_PROFILES_FILE
    Error when a named file does not exist or is not valid XML. Comma-separated lists and
    file:// prefixes are understood; inline XML is accepted as is. Warning when the file
    belongs to a different middleware than the one that will run, because it is then
    ignored (a Cyclone DDS configuration while Fast DDS, the default, is active). Warning
    when a Fast DDS profile allows only shared-memory transport: nodes on other machines,
    or in containers without a shared /dev/shm, will not connect.

ZENOH_SESSION_CONFIG_URI, ZENOH_ROUTER_CONFIG_URI
    Error when the file does not exist: rmw_zenoh_cpp stops initializing with "Invalid
    configuration file". The JSON5 content is not checked.

ZENOH_CONFIG_OVERRIDE, ZENOH_ROUTER_CHECK_ATTEMPTS, ZENOH_SHM_ALLOC_SIZE,
ZENOH_SHM_MESSAGE_SIZE_THRESHOLD
    Warning for values rmw_zenoh_cpp cannot use. An override pair must be key=value with a
    JSON5 value: mode="peer" works, mode=peer is ignored. A router check count that is not
    a number is read as 0, which waits for a router forever. Shared-memory sizes must be
    positive numbers, and the message size threshold a power of 2. Every Zenoh variable
    gets a warning when the middleware is not rmw_zenoh_cpp, because it is then ignored.

Zenoh router
    When the middleware is rmw_zenoh_cpp with the default session configuration, doctor
    tries a TCP connection to 127.0.0.1:7447, where that configuration expects the router.
    Warning when nothing listens: nodes start, but do not discover each other until a
    router runs (`ros2 run rmw_zenoh_cpp rmw_zenohd`). Skipped when a session configuration
    file or an override of mode or connect is set, or ZENOH_ROUTER_CHECK_ATTEMPTS is
    negative.

## Inside ros2 doctor

When rosfix is installed from PyPI (`pip install rosfix`, from v0.2.3), the same checks also run as part of
`ros2 doctor`: an error here counts as a failed ros2doctor check, a warning as a warning, and
`ros2 doctor --report` gets a ROSFIX ENVIRONMENT section listing every check. The Python
package registers them through ros2doctor's `ros2doctor.checks` and `ros2doctor.report` entry
points and runs `rosfix doctor --format json`, which is also available directly for scripts.

## What it does not check

doctor reads variables and files. Apart from the one local connection attempt to a Zenoh
router, it does not start a participant, send packets or look at firewalls and network
interfaces. A clean doctor report means the configuration is
consistent, not that discovery works on your network.
