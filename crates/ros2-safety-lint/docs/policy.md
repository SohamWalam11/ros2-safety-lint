# rosfix policy

    rosfix policy [PATH] [--launch FILE] [-o policy.xml]

Writes an SROS2 access-control policy for the launch graph, before anything runs.

The usual way to get a policy is `ros2 security generate_policy`, which records the graph of
a running system: it needs the robot running, and it misses whatever did not happen while it
watched. rosfix already knows, from the launch files and the source code, which node
publishes, subscribes, serves, calls and executes what, after namespaces and remaps, for C++
and Python nodes. `rosfix policy` turns that into a policy file.

## What is written

The format is sros2's policy format 0.2.0, the input of
`ros2 security create_permission <keystore> <enclave> <policy file>`:

    one enclave per node: the node's --enclave if the launch file sets one, else its fully
    qualified name
    one profile per node, with the node's namespace and name
    topics publish / subscribe, services reply / request and actions execute / call: exactly
    the names the node's code uses, after namespaces and remaps
    the access every node needs, written inline rather than as an include of sros2's
    common/node.xml: parameter services, /parameter_events, rosout, /clock and the type
    description service

With --launch FILE only the nodes of that launch file's graph are included (matched by the end
of the path).

## What is not guessed

A node whose interface rosfix could not fully determine gets a comment in its profile,
saying why: its source code was not found (an installed package), or some of its names are
computed at runtime (plugins, names built from parameters). Such a profile allows only what
rosfix knows; complete it with `ros2 security generate_policy` on the running node. The
number of such profiles is printed on stderr. Nodes whose name is not known statically get no
profile and are listed in a comment at the top.

Policies generated for demos, navigation2 and turtlebot3 validate against sros2's
policy.xsd.
