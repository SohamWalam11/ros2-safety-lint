// rosfix for VS Code: starts `rosfix lsp` for the workspace and shows its findings and
// quick fixes. All analysis happens in the rosfix executable.
'use strict';

const vscode = require('vscode');
const { LanguageClient } = require('vscode-languageclient/node');

let client;

// Files rosfix reads. Plain XML and YAML are included because launch, URDF, parameter and
// security files often have generic extensions.
const DOCUMENTS = [
  { scheme: 'file', language: 'python' },
  { scheme: 'file', language: 'cpp' },
  { scheme: 'file', language: 'xml' },
  { scheme: 'file', language: 'yaml' },
  { scheme: 'file', pattern: '**/*.{launch,urdf,xacro,srdf,rviz}' },
  { scheme: 'file', pattern: '**/{CMakeLists.txt,package.xml,setup.py,setup.cfg}' },
];

function start(context) {
  const config = vscode.workspace.getConfiguration('rosfix');
  const folder = vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0];
  if (!folder) {
    return;
  }
  const command = config.get('path', 'rosfix');
  const args = ['lsp', folder.uri.fsPath, ...config.get('extraArgs', [])];
  client = new LanguageClient(
    'rosfix',
    'rosfix',
    { command, args, options: { cwd: folder.uri.fsPath } },
    { documentSelector: DOCUMENTS, workspaceFolder: folder }
  );
  client.start().catch((err) => {
    vscode.window.showErrorMessage(
      `rosfix: could not start \`${command} lsp\` (${err.message}). ` +
        'Install rosfix (cargo install rosfix) or set rosfix.path.'
    );
  });
  context.subscriptions.push(client);
}

function activate(context) {
  start(context);
  context.subscriptions.push(
    vscode.commands.registerCommand('rosfix.restart', async () => {
      if (client) {
        await client.stop();
        client = undefined;
      }
      start(context);
    }),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration('rosfix.path') || e.affectsConfiguration('rosfix.extraArgs')) {
        vscode.commands.executeCommand('rosfix.restart');
      }
    })
  );
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
