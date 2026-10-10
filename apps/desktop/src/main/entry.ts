import { app } from 'electron';

// This fixed app-owned entrypoint works with RunAsNode disabled in signed builds.
// It cannot load a caller-supplied script and never starts the editor or its stores.
if (process.argv.includes('--powermove-mcp')) {
  void import('./agent-tools/external-entry').then(module => module.runExternalMcpEntry()).catch(error => {
    process.stderr.write(`Powermove MCP failed: ${String(error)}\n`);
    app.exit(1);
  });
} else if (process.argv.includes('--powermove-agent-tools')) {
  if (process.platform === 'darwin') app.setActivationPolicy('prohibited');
  void import('./agent-tools/mcp-server.mjs').catch(error => {
    process.stderr.write(`Powermove tool bridge failed: ${String(error)}\n`);
    app.exit(1);
  });
} else {
  void import('./index');
}
