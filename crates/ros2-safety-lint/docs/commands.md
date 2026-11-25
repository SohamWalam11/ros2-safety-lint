# Commands

    rosfix [PATH] [OPTIONS]          same as rosfix check
    rosfix <COMMAND> [ARGS]

PATH is a workspace, a src directory or a single package; it defaults to the current
directory. `rosfix <COMMAND> --help` lists every option of a command.

The original single-command form keeps working unchanged: `rosfix src --fix --write`,
`rosfix --print-graph`, `rosfix --list-rules` and the other flags behave as they always
did. A directory that happens to be named like a command is analysed with `rosfix ./check`.

## check

    rosfix check [PATH] [OPTIONS]

Analyses the workspace and prints the findings.

    --format FORMAT            text, json, sarif or rdjson (default text)
    -o, --output FILE          write the report to FILE instead of stdout
    --fail-on LEVEL            error, warning, info or never; exit 1 when a finding at
                               or above LEVEL remains (default: fail_on in rosfix.toml,
                               else warning)
    --baseline FILE            hide findings recorded in FILE (see baseline)
    --distro NAME              humble, jazzy, kilted or rolling (default: ROS_DISTRO)
    --ros-prefix DIR           installed ROS prefix to read, repeatable (default:
                               AMENT_PREFIX_PATH)
    --no-ros                   ignore the installed ROS environment
    --agents a,b               run only these agents (core always runs)
    --config FILE              configuration file (default: PATH/rosfix.toml)
    --assist                   let the fix assistant pick repairs (see assist)
    --online                   attach references from public sources, and build local packs
                               for launched packages no pack covers (see assist)
    --show-queries             with --online, print each query before sending it
    --stats                    per-agent timing and indexing statistics on stderr
    --jobs N                   index on N threads (needs the parallel feature)
    -v, --verbose              also print each finding's rule summary
    --no-color                 plain output (also when NO_COLOR is set)

With the text format the summary goes to stderr; with json or sarif, stdout holds only
the report.

## fix

    rosfix fix [PATH] [OPTIONS]

Computes the automatic fixes, verifies each one by re-analysing the patched workspace in
memory, and prints the verified fixes as a unified diff. Nothing is written.

    --write            write the verified fixes to disk
    --verify-build     also build the workspace before and after the fixes in a
                       temporary copy, and refuse fixes that break a working build
    --assist           include the fix assistant's choices in the diff
    --accept-assist    with --write, also write the assistant's choices

All options of check are accepted as well. With `--format json` or `sarif` the diff goes
to stderr. Findings fixed on disk no longer count for the exit code. See
`rosfix docs fixes` for how verification works.

## watch

    rosfix watch [PATH] [--interval-ms N]

Keeps running and re-analyses whenever a file changes, printing findings that appear
(marked +) and disappear (marked -). Only changed files are parsed again and only agents
that read them run again, so a re-analysis usually takes a fraction of a second. Stop it
with Ctrl-C.

## lsp

    rosfix lsp [PATH]

Runs a language server on stdin and stdout. It publishes diagnostics for open files,
including unsaved edits. See `rosfix docs editors`.

## graph

    rosfix graph [PATH] [--format text|json|dot|mermaid|callbacks] [-o FILE]

Prints the resolved launch graph: one section per launch file that can be started on its
own, with every node instance, its package and executable, whether its source code was
found, and how many parameters were bound to it.

    text      one line per node; "+?" after the parameter count means some parameter
              sources could not be read, "(partial)" marks launch files with dynamic parts
    json      nodes and the topics connecting them, for scripts
    dot       Graphviz: dot -Tsvg graph.dot -o graph.svg
    mermaid   a flowchart that renders on GitHub and in most Markdown viewers
    callbacks the static callback graph as JSON (see below)

In dot and mermaid, nodes are boxes and topics are rounded shapes; an arrow goes from each
publisher to the topic and from the topic to each subscriber. Topic names are the final
names after namespaces and remaps. Conditional nodes are drawn dashed (dot) or labelled.

The callbacks format describes what each launched node executes, as read from its source:
the executors its code creates, its callback groups (mutually exclusive or reentrant, and
whether a group is spun by a dedicated executor), and its callbacks. A timer callback has
`period_ms` when the period is written as a literal (`100ms`,
`std::chrono::milliseconds(100)`, `create_timer(0.1, ...)`, `1s / 30`), else null. A
subscription callback has the resolved topic it listens to. `edges` connects each publisher
to the subscription callbacks of the same topic in the same launch graph. The result is a
starting point for response-time analysis or for CARET path definitions; nothing in it is
measured, and callbacks that code creates dynamically are missing.

## rules and explain

    rosfix rules
    rosfix explain P003

`rules` lists every rule with its agent, default severity and name. `explain` prints the
summary and the full explanation of one rule. `rosfix docs rules` prints all explanations.

## baseline

    rosfix baseline [PATH] [-o FILE]

Records a fingerprint of every current finding in FILE (default rosfix-baseline.json) and
exits 0. Later runs with `--baseline FILE` report only findings that are not in it. A
fingerprint is the rule, the file and the message, without line numbers, so findings
survive unrelated edits above them. This is the usual way to adopt rosfix on an existing
code base: record once, commit the file, fix the old findings over time.

## init

    rosfix init [PATH] [--distro NAME] [--no-ci] [--pre-commit] [--force] [--dry-run]

Sets up a repository:

    rosfix.toml                   a starting configuration
    .github/workflows/rosfix.yml  a GitHub Actions job (skip with --no-ci)
    .pre-commit-config.yaml       only with --pre-commit

Files that already exist are kept unless `--force` is given. An existing
.pre-commit-config.yaml is never edited, even with `--force`; the entry to add is printed
instead. `--dry-run` shows what would be written. The distribution recorded in rosfix.toml
comes from `--distro`, else ROS_DISTRO, else it is left commented out.

## doctor

    rosfix doctor [--format text|json]

Checks the ROS 2 environment of the current shell: distribution, install prefixes,
middleware, domain ID, discovery settings and DDS configuration files. Exits 1 if it finds
an error. With the PyPI package installed, the same checks also run inside `ros2 doctor`.
See `rosfix docs doctor`.

## migrate

    rosfix migrate [PATH] --to NAME [--from NAME] [--fix [--write]]

Analyses the workspace against the current distribution and against the target, and
reports only the findings that are new on the target. See `rosfix docs migrate`.

## triage

    rosfix triage LOG [PATH] [--format text|json] [-o FILE]

Reads a launch or node log (`-` for stdin), finds the error lines it recognises, and lists
for each one the workspace findings that explain it. Accepts the analysis options of check.
Always exits 0. See `rosfix docs triage`.

## check-bag

    rosfix check-bag BAG [PATH] [--launch FILE] [--format text|json] [-o FILE]

Compares the launch graph with a recording: a rosbag2 directory or metadata.yaml, or
`ros2 topic list -t` output. Reports type mismatches, QoS differences, recorded topics
without a static publisher and, with --launch, published topics missing from the recording.
Exits 1 on a type mismatch. See `rosfix docs check-bag`.

## status

    rosfix status [PATH] [--format text|json]

Reports, for every agent, whether it ran, what it read and checked, its findings, and the
next actions that would let it check more. See `rosfix docs status`.

## report

    rosfix report [PATH] [-o report.html]

Writes one HTML page with everything about the workspace: counts of findings, packages,
files, launch graphs and nodes; every finding grouped by file, with its help and fix; the
agent table from `status`; and the launch graph as a Mermaid diagram. The page is self
contained except for mermaid.js, loaded from jsDelivr to draw the graph; offline, the
graph is shown as its Mermaid source. Above 150 node instances the graph is left out. The
page follows the system's light or dark setting. Useful as a CI artifact:

    rosfix report . -o rosfix-report.html

## policy

    rosfix policy [PATH] [--launch FILE] [-o policy.xml]

Writes an SROS2 access-control policy (format 0.2.0) from the launch graph: one enclave and
profile per node, allowing exactly the topics, services and actions its code uses. Profiles
rosfix could not complete are marked with a comment. See `rosfix docs policy`.

## nodl export

    rosfix nodl export [PATH] [-o DIR]

Writes a NoDL (Node Definition Language) document for each workspace executable from what
rosfix read in its source, to stdout or as DIR/<package>/<executable>.nodl.yaml. Whatever is
not known statically is written as a `# ?` comment, not guessed. See `rosfix docs nodl`.

## shell

    rosfix shell [PATH]

An interactive prompt for working on one workspace: type the commands of this page without
the `rosfix` prefix (`status`, `check -v`, `fix --write`, `graph --format mermaid`, ...), and
they run on the current workspace. A typo or a failing command does not end the shell; exit
codes are shown instead. Shell commands: `cd PATH` changes the workspace, `pwd` shows it,
`history` lists the commands typed so far, `help` lists all this, and `exit`, `quit` or
end of input (Ctrl-D, or Ctrl-Z then Enter on Windows) leaves. `watch` runs until Ctrl-C,
which also ends the shell, and `lsp` is for editors, so it is not available here.

On a terminal the prompt has line editing: Up and Down walk the history, which is kept in
~/.rosfix_history between sessions, Ctrl-R searches it, Ctrl-C clears the line, and Tab
completes command names, rule ids after `explain`, topics after `docs`, values after
`--format`, and file paths. When the input is a pipe or a file, lines are read as they are,
so scripts can drive the shell.

The shell is optional. Every command works the same from PowerShell, cmd, bash or zsh.

## docs

    rosfix docs            list the topics
    rosfix docs TOPIC      print one topic
    rosfix docs all        print the whole manual

The manual is compiled into the binary and needs no network.

## Exit codes

    0   no finding at or above the --fail-on level (or the command succeeded)
    1   at least one such finding remains; for doctor, an error was found
    2   usage error, unreadable configuration, or another tool error
