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
