# Continuous integration

## GitHub Actions

    rosfix init

writes .github/workflows/rosfix.yml, which runs on every push and pull request:

    - uses: actions/checkout@v4
    - uses: SohamWalam11/ros2-safety-lint@v1
      with:
        path: .
        fail-on: error

The action downloads the release binary for the runner, checks it against the release's
SHA256SUMS, runs rosfix with `--format sarif`, uploads the result to GitHub code scanning
(findings then appear on the pull request diff), and fails the job according to
`fail-on`. Inputs:

    path           directory to analyse (default .)
    fail-on        error, warning, info or never (default error)
    distro         humble, jazzy, kilted or rolling, to enable the knowledge pack
    version        release tag, or latest (default)
    args           extra arguments passed to rosfix
    upload-sarif   "false" to skip the code scanning upload
    pr-suggestions "true" to post review comments on pull requests (see below)

The job needs `security-events: write` permission for the upload.

## Suggested changes on pull requests

With `pr-suggestions: "true"`, the action also posts a review comment on every finding on
a line the pull request changed. When the finding has a fix that passes the same
verification as `rosfix fix` (re-analysis finds no new error and no new finding on the
edited span), the comment carries it as a GitHub suggested change, which the author can
apply with one click. The job needs `pull-requests: write` permission:

    permissions:
      contents: read
      security-events: write
      pull-requests: write

The comments are posted by reviewdog from `rosfix --format rdjson`, which any other CI
system with reviewdog can use the same way:

    rosfix . --format rdjson --fail-on never | reviewdog -f=rdjson -reporter=github-pr-review

Fixes that edit more than one file are reported without a suggestion, since a suggested
change can only replace lines of the commented file.

Without the action, any job can run the binary directly:

    rosfix . --format sarif -o rosfix.sarif --fail-on error

## pre-commit

    rosfix init --pre-commit

writes a .pre-commit-config.yaml with this entry, or prints it if the file already
exists:

    - repo: https://github.com/SohamWalam11/ros2-safety-lint
      rev: v0.2.3
      hooks:
        - id: rosfix

The `rosfix` hook installs rosfix from PyPI in pre-commit's own environment, so no Rust
toolchain is needed. The `rosfix-system` hook uses a rosfix already on PATH. Both analyse
the whole repository, because a change in one file can break another (a parameter file
and the node that reads it), and run only when a ROS-relevant file changed.

## Adopting rosfix on an existing code base

Record the current findings once and commit the file:

    rosfix baseline -o rosfix-baseline.json

Then let CI report only new findings:

    rosfix check --baseline rosfix-baseline.json

The text summary says how many findings the baseline hid. Remove entries as they are
fixed, or regenerate the file.

## Machine-readable output

SARIF 2.1.0 (`--format sarif`) is the format GitHub code scanning reads, and many other
review tools read it too. JSON (`--format json`) is described in `rosfix docs output`. Both keep
stdout free of anything but the report.
