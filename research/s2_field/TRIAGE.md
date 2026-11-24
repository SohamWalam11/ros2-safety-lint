# Study 2 triage guide

Each item is one rosfix finding on a repository's Jazzy release branch: rule, message, file,
line, and a source excerpt. The full clone is at `d:\Rust OSS\research\.cache\head\<owner>__<name>\`
(read any file there you need: package.xml, launch files, the node source, the YAML).
`rosfix --explain <RULE>` text is summarised in the rule table of `d:\Rust OSS\README.md`.

Decide whether the finding is correct **for this repository as it would be built and launched
on ROS 2 Jazzy**.

| `verdict` | Meaning |
|---|---|
| `tp` | Real problem: the configuration really is wrong (ignored, mismatched, missing, crashing) in normal use. |
| `tp_intentional` | The statement is true, but the file is a deliberate negative example: a test fixture, a "broken on purpose" demo, a template. |
| `fp` | rosfix is wrong: the configuration works (e.g. the parameter *is* declared, through a base class, a plugin, `allow_undeclared_parameters`, a library; the dependency *is* provided; the file *is* installed). |
| `unsure` | Cannot decide from the repository alone (needs an external package's source, runtime behaviour, or a design intent you cannot see). |

Also give `fp_cause` (for `fp`, ≤ 15 words: what rosfix missed) and `rationale` (≤ 30 words).
Look for evidence; do not guess. A parameter "undeclared" claim is `fp` only if you find the
declaration (or a mechanism that accepts it) in the node's code or its base classes.
