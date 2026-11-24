# rosfix status

    rosfix status [PATH] [--format text|json]

Analyses the workspace and, instead of listing findings, reports on each agent: what it did,
what it found, and what currently stops it from checking more.

For every agent:

    state      ran (with its time), reused (inputs unchanged since the previous run in
               `watch` or the language server), idle (nothing in the workspace for it to
               check) or not selected (left out with --agents)
    reads      the workspace files of the kinds the agent depends on, by kind
    checked    what it worked on: parameter files and nodes with bound parameters, topic
               endpoints, source files, robot descriptions, launch graphs, packages
    findings   its findings per rule, with the number of errors, warnings and infos
    next       the concrete next actions, derived from this workspace

The next actions name what limits the agent and what would lift the limit:

    no ROS distribution set: the knowledge packs are off (pass --distro, set `distro` in
    rosfix.toml, or source a ROS environment)
    launched nodes from installed packages that no knowledge pack or NoDL document
    describes, by package
    launch graphs with dynamic parts, where whole-graph checks stay silent (`rosfix graph`
    shows which)
    nodes whose interface is only partly known, and endpoints whose QoS is not known
    statically (`rosfix check-bag` compares with a recording)
    nodes that accept parameters rosfix cannot enumerate (plugins)
    launched nodes not found anywhere (source the install space or pass --ros-prefix)
    findings with verified fixes (`rosfix fix --write`) and findings the fix assistant can
    repair (`rosfix fix --assist`)

The header gives the size of the analysis (packages, files parsed, launch graphs, node
instances), the ROS distribution in use and how many installed prefixes were read.
`--format json` prints the same information for scripts and dashboards.
