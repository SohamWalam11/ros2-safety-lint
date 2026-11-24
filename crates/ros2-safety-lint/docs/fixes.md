# Automatic fixes

    rosfix fix              show verified fixes as a diff
    rosfix fix --write      write them

Some findings carry a fix: a small, local text edit that removes the problem. Examples are
writing `1.0` for an integer given to a double parameter, renaming a misspelled
`ros__parametres` key, adding a missing `<depend>` to package.xml, or installing a launch
directory that CMakeLists.txt never installs.

## Three kinds of fix

    safe        mechanical and local; written by `fix --write`
    suggestion  shown in the report as a manual fix; never written
    assisted    chosen by the fix assistant among valid options; written only with
                `--write --accept-assist` (see `rosfix docs assist`)

The text report labels them "fix (auto)" and "fix (manual)". In the JSON report each fix
has a "safety" field and its byte-range edits.

These rules have safe fixes when the right replacement is unambiguous:

    P001  parameter file key that matches no node: renamed to the node's name
    P002  misspelled ros__parameters level: renamed
    P003  undeclared parameter that is a near miss of a declared one: renamed
    P004  integer written for a double parameter: 1 becomes 1.0; a number or word
          written for a string parameter: quoted
    L002  dependency used but not declared: <depend> added to package.xml
    L003  setup.cfg without install_scripts: section added
    L004  launch or config directory never installed: install rule added
    N002  Nav2 plugin type typo: corrected
    N003  frame name that is a near miss of a URDF frame: corrected
    N006  Nav2 parameter name typo: corrected
    C003  controller type typo: corrected
    F003  frame name with a leading slash: slash removed

When several options are equally close, rosfix does not guess. The finding then carries
its candidates for the fix assistant instead.

## How a fix is verified

1. Synthesize. All safe fixes are collected from the findings. Duplicates and fixes whose
   edits overlap an earlier fix are dropped.
2. Verify. The fixes are applied in memory and the whole workspace is analysed again. A
   fix is kept only if its finding is gone and no new error-level finding appears in a
   file it touched. If any fix fails, it is dropped and the rest are verified again, up
   to three rounds.
3. Diff. The kept fixes are shown as a unified diff.
4. Apply. With `--write`, each changed file is written to a temporary file in the same
   directory and renamed over the original, so a crash never leaves a half-written file.

The re-analysis is incremental: only the patched files are parsed again, and only agents
that read those kinds of files run again. On Navigation2 (732 files) a full fix run takes
two to three seconds.

Fixes that were dropped are listed on stderr with the reason, for example "the finding is
still reported after the fix".

## Building before writing

    rosfix fix --write --verify-build

copies the workspace to a temporary directory twice, unchanged and with the fixes, and
builds both copies: in a `ros:<distro>` container without network access when Docker is
available, otherwise with the local colcon. If the unchanged copy builds and the fixed
copy does not, nothing is written and rosfix exits with code 2. If the unchanged copy
does not build either, the result is reported as inconclusive and the fixes are written.

## Exit code after fixing

Findings fixed on disk by `--write` no longer count toward `--fail-on`. A CI job can
therefore run `rosfix fix --write` and fail only on what remains.

## Distribution changes

`rosfix migrate --to NAME --fix` offers the same verified fixes for problems that appear
on another distribution, verified against that distribution's knowledge pack. See
`rosfix docs migrate`.
