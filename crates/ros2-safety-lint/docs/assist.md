# Fix assistant and online references

## The fix assistant (--assist)

Some findings have a closed set of valid answers but no single obvious one. A parameter
written as `max_lin_vel` where the node declares `max_linear_velocity`, `max_accel` and
`wheel_radius` is an example. rosfix attaches those valid values to the finding: declared
parameters, ros2_control joints, URDF frames, executables of the package, enclaves in the
keystore.

    rosfix check --assist
    rosfix fix --assist
    rosfix fix --assist --write --accept-assist

With `--assist`, rosfix picks one of the values for each such finding, in this order:

1. a deterministic template, when the agent already knows the answer;
2. lexical ranking: edit distance and word overlap, with common ROS abbreviations
   expanded (vel, accel, freq, tol, cmd, ctrl and others);
3. a learned ranker that re-scores the lexical candidates;
4. no choice at all, only the explanation, when no candidate clearly wins.

The learned ranker computes 16 string features per candidate and scores them with a small
neural network (16 inputs, 16 hidden units, 1 output; 321 numbers including the input
normalisation, about 1 KB). The
weights are compiled into the binary. There are no model files, no ML runtime, no GPU and
no network access. Scoring a finding takes microseconds. It is trained by the
rosfix-train tool on misspellings of real parameter names from the knowledge packs, and
some training cases leave the right answer out on purpose, so it learns to abstain.

The assistant can only choose among real options. It never writes text that is not one
of them. Every choice is verified by re-analysis like any other fix. Verification shows
that the finding goes away, not that the choice is what you meant, so assisted fixes are
written only with `--accept-assist`. Review the diff first.

## Online references (--online)

Off by default. With `--online`, or `online = true` in rosfix.toml, findings get links to
related Robotics Stack Exchange questions and GitHub issues, and to results from your own
SearXNG instance if `searxng_url` is set.

What is sent: a query built only from the rule that fired, its name and summary. No file
content, path, node, package or topic name leaves the machine. `--show-queries` prints
each query before it is sent.

How it is sent: by a separate helper process that only talks to an allowlist of HTTPS
hosts, follows no redirects, uses short timeouts and size limits, and caches results
locally. Builds without the online feature print a note and skip the lookup.

## Local packs (--online)

With `--online` and a known distribution (`--distro`, rosfix.toml or ROS_DISTRO), launched
packages that are neither in the workspace nor in the distribution's knowledge pack get a
local pack:

1. The rosdistro index of the distribution is downloaded and cached for a week.
2. Each package is looked up in it locally. Packages it does not release are noted and
   skipped until the index is refreshed.
3. The source archive of the package's repository is downloaded from GitHub at the
   distribution's branch, at most 8 repositories per run.
4. rosfix's extractors build a pack from it, stored in `<cache>/rosfix/packs/<distro>/`
   (`ROSFIX_CACHE_DIR` overrides the cache root). The analysis is repeated with the new pack.

Later runs read local packs without any network access. A local pack adds packages but never
replaces what the embedded pack says. Checks that compare distributions use only the embedded
packs, so every distribution is compared with the same coverage.

What is sent: the rosdistro index URL and the archive URLs of public repositories listed in
it (`--show-queries` prints them). Private package names never leave the machine.

Safety: only raw.githubusercontent.com/ros/rosdistro and codeload.github.com archives are
allowed. Archives are limited in size, unpacked with path checks (no `..`, absolute paths or
links), and only source, build and manifest files are kept. Nothing downloaded is executed,
and the unpacked sources are deleted once the pack is built.
