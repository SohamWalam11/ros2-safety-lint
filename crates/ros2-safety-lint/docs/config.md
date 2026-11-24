# Configuration

rosfix reads rosfix.toml from the directory it analyses (an older ros2-safety.toml is also
accepted). `--config FILE` uses another file. Every key is optional; `rosfix init` writes
a starting file.

    distro = "jazzy"
    fail_on = "warning"
    ignore = ["third_party/**"]
    test_paths = ["**/test/**", "**/tests/**"]
    external_topics = ["/scan", "/camera/image_raw"]
    online = false
    searxng_url = "https://searx.example.org"

    [rules]
    P003 = "off"
    Q004 = "warning"

Unknown keys, unknown rule IDs and invalid values are errors. rosfix stops with exit code
2 instead of quietly ignoring a misspelled setting, which would be the kind of mistake it
exists to catch.

## Keys

distro
    The ROS distribution of the workspace: humble, jazzy, kilted or rolling. It selects
    the knowledge pack for installed packages (see `rosfix docs environment`). The
    command line option `--distro` takes precedence, then this key, then ROS_DISTRO.

fail_on
    info, warning or error. rosfix exits with code 1 when a finding at or above this
    severity remains. The default is warning. `--fail-on` overrides it and also accepts
    never.

ignore
    Glob patterns, relative to the analysed directory, for paths to skip. Directories
    named .git, build, install, log, target, node_modules, __pycache__ and virtual
    environments are always skipped, and so is every package that contains a
    COLCON_IGNORE or AMENT_IGNORE file.

test_paths
    Glob patterns for test code. The executor rules do not report blocking calls in
    these files, because tests often wait on futures on purpose. The default covers
    test and tests directories and files named test_* or *_test in C++ and Python.
    Setting the key replaces the default list.

external_topics
    Topics that are published or subscribed outside the workspace: drivers, simulators,
    other robots. Topic rules do not report them as unconnected. /tf, /tf_static,
    /clock, /rosout, /parameter_events, /diagnostics, /joy, /cmd_vel, /initialpose,
    /goal_pose and /clicked_point are always treated as external. Entries are added to
    that list.

online
    true allows `--online` lookups without passing the option each time. Default false.
    See `rosfix docs assist`.

searxng_url
    A self-hosted SearXNG instance to include in online lookups.

## Rule settings

The [rules] table sets each rule by ID:

    off                  the rule is not reported
    on                   the rule is reported at its default severity
    info, warning, error the rule is reported at that severity

Rules marked "off by default" in `rosfix rules` are enabled with "on" or a severity.

## Suppressing one finding

A comment containing `rosfix: ignore` on the same line as a finding, or on a line of
its own directly before it, hides that finding. A comment at the end of a line of code
applies to that line only. List rule IDs to hide only those:

    max_vel: 1   # rosfix: ignore[P004]

    <!-- rosfix: ignore[L002, L004] -->
    <depend>my_vendor_pkg</depend>

    // rosfix: ignore[E001]
    future.wait();

Any comment syntax works, because rosfix only looks for the text. Without a list, every
finding on that line is hidden. Prefer the list form, so a new kind of problem on the
same line is still reported.
