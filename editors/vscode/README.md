# rosfix for VS Code

Shows rosfix findings in the editor while you type, and offers its fixes as quick fixes.
The extension only starts `rosfix lsp`; every check runs in the rosfix executable, so the
results are the same as `rosfix check` on the command line.

## Requirements

rosfix on PATH, installed with `cargo install rosfix` or `pip install rosfix`. To use a
different executable, set `rosfix.path`.

## What you get

- Findings for launch files, parameter YAML, C++ and Python nodes, package manifests,
  CMakeLists.txt, URDF/xacro, SRDF, ros_gz_bridge and security files, updated on every
  edit, including unsaved changes. Only the edited file is parsed again.
- Quick fixes (the light bulb, or Ctrl+.) for findings that have a fix. Fixes that
  `rosfix fix --write` would apply on its own are marked preferred.

The extension starts when the workspace contains a package.xml.

## Settings

    rosfix.path          the rosfix executable (default: rosfix)
    rosfix.extraArgs     extra options for `rosfix lsp`, e.g. ["--distro", "jazzy"]
    rosfix.trace.server  log the protocol messages: off, messages or verbose

Run "rosfix: Restart language server" from the command palette after installing a new
rosfix version.

## Building the package

    cd editors/vscode
    npm install
    npx @vscode/vsce package

This produces `rosfix-<version>.vsix`, which can be installed with "Extensions: Install from
VSIX...". Set your own `publisher` in package.json before publishing to the Marketplace.
