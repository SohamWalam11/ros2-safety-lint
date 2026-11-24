# rosfix migrate

    rosfix migrate [PATH] --to kilted
    rosfix migrate [PATH] --from humble --to jazzy --fix
    rosfix migrate [PATH] --to jazzy --fix --write

Upstream packages rename, remove and retype parameters between distributions. Nav2's
behavior server, for example, read `costmap_topic` on Humble and reads
`local_costmap_topic` on Jazzy. A configuration that sets the old name keeps loading
without an error after the upgrade; the value is simply ignored.

migrate analyses the workspace twice: once with the knowledge pack of the current
distribution and once with the pack of the target. It reports only the findings that are
new on the target. Typical results are parameters that the target no longer declares
but another distribution does (P007, with the closest current name in the help text),
other undeclared parameters (P003), values whose declared type changed (P004), and
executables, plugins or components that no longer exist.

The current distribution comes from `--from`, else `distro` in rosfix.toml, else
ROS_DISTRO. migrate stops with an error if none is set, and when both distributions use
the same pack (kilted and rolling).

## Output

The report has the same formats as check (`--format text|json|sarif`, `-o FILE`). A
summary on stderr gives three numbers: findings new on the target, findings already
present on the current distribution (these are not migration problems and are not shown),
and findings that the target no longer reports.

The exit code follows `--fail-on` (default: fail_on in rosfix.toml, else warning) and
counts only the new findings, so `rosfix migrate --to jazzy` can gate an upgrade branch
in CI.

## Fixes

With `--fix`, the safe fixes of the new findings are verified against the target
distribution and shown as a diff; `--write` applies them. A renamed parameter is fixed
only when the target declares exactly one close match; otherwise the finding lists the
candidates and is left to you.

## Limits

migrate knows only what the knowledge packs contain (see `rosfix docs environment`).
Changes in packages outside the packs, changed defaults, changed behaviour behind an
unchanged parameter name, and C++ or Python API changes are not detected. The installed
ROS environment is ignored during migrate, because it belongs to a single distribution
and would make both runs see the same packages.
