# Editors and watch mode

## Language server

    rosfix lsp [PATH]

speaks the Language Server Protocol on stdin and stdout. It handles initialize,
textDocument/didOpen, didChange, didSave and didClose, codeAction, and shutdown, and
publishes diagnostics for the workspace, including the unsaved text of open files. Each
edit is a change event for the incremental engine: only the edited file is parsed again
and only the agents that read that kind of file run again.

Findings that have a fix come with a quick fix (codeAction, kind quickfix) that applies
the fix's edits. Fixes that `rosfix fix --write` would apply on its own are marked
preferred; suggested and assisted fixes are offered too, since choosing one in the
editor is a deliberate act.

Any editor with a generic LSP client can use it. For Neovim:

    vim.lsp.start({
      name = "rosfix",
      cmd = { "rosfix", "lsp" },
      root_dir = vim.fs.root(0, { "rosfix.toml", ".git" }),
    })

## VS Code

The repository has a VS Code extension in editors/vscode. It starts `rosfix lsp` for the
workspace folder when the folder contains a package.xml, and shows findings and quick fixes
in launch, parameter, source, manifest, URDF, SRDF and bridge files. Build and install it:

    cd editors/vscode
    npm install
    npx @vscode/vsce package
    code --install-extension rosfix-0.2.3.vsix

Settings: rosfix.path (the executable, default rosfix), rosfix.extraArgs (extra options for
`rosfix lsp`, such as ["--distro", "jazzy"]) and rosfix.trace.server.

## Watch mode

    rosfix watch [PATH] [--interval-ms 500]

polls the workspace for changes and, after each change, prints the findings that
appeared (prefixed with +) and those that disappeared (prefixed with -). It is a quick way
to see the effect of an edit without an editor integration. Stop it with Ctrl-C.
