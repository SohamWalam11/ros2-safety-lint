# Output formats

## Text

The default. One block per finding:

    warning[C003]: controller `jtc` has type `joint_trajectory_controler/JointTrajectoryController`, which is not a known controller plugin; did you mean `joint_trajectory_controller/JointTrajectoryController`?
      --> arm/config/controllers.yaml:6:13
         6 |       type: joint_trajectory_controler/JointTrajectoryController
      fix (auto): Use `joint_trajectory_controller/JointTrajectoryController`

The first line has the severity, the rule ID and the message. The location is
file:line:column, relative to the analysed directory, followed by the source line.
Related locations (note:), help and fixes follow on indented lines; `--verbose` adds the
rule's one-line summary. A finding inside a launch file that other launch files include
also names the include statements ("`child.launch.xml` is included here", up to three), so
you can see which top-level launch file runs into it.
The summary line (counts per severity, findings hidden by suppressions and the baseline,
and how many safe fixes are available) goes to stderr. Colour is used only on a terminal
and is turned off by `--no-color` or the NO_COLOR variable.

## JSON

`--format json` prints one object:

    version       format version, currently 1; it changes only on incompatible changes
    tool          {"name": "rosfix", "version": "..."}
    diagnostics   list of findings
    agents        per agent: id, title, ran, reused, diagnostics, micros
    timings_ms    index_ms, resolve_ms, agents_ms

Each finding has:

    rule, name, agent, severity, message
    location      file, line, column, end_line, end_column, start_byte, end_byte
    related       list of {message, location}
    help          string or null
    references    list of {title, url} (only with --online)
    fix           null, or {description, safety, edits}; safety is safe, suggestion or
                  assisted; each edit has file, start_byte, end_byte, replacement
    fingerprint   the stable ID used by baselines

Lines and columns start at 1. Byte offsets refer to the file as read from disk.

## SARIF

`--format sarif` writes SARIF 2.1.0 with one run. Every rule is listed in the tool's
rule table with its summary, explanation, default level and agent. Results carry the
message (with help and references appended), the location, related locations, fixes as
SARIF fix objects, and a partial fingerprint `rosfix/v1` equal to the JSON fingerprint, so
GitHub code scanning tracks a finding across commits.

## rdjson

`--format rdjson` writes the Reviewdog Diagnostic Format: one diagnostic per finding with
message (help appended), path, range (1-based lines, 1-based byte columns), severity and
the rule as `code`. Paths are relative to the current directory, so reviewdog can match
them to a diff. A finding whose fix passes verification (the same re-analysis as
`rosfix fix`) and edits only its own file carries the fix as `suggestions`; reviewdog's
github-pr-review reporter turns those into suggested changes. Producing rdjson therefore
takes as long as `rosfix fix` without `--write`. See `rosfix docs ci`.

## Launch graph

`rosfix graph` has its own formats: text, json, dot, mermaid and callbacks. See
`rosfix docs commands`.
