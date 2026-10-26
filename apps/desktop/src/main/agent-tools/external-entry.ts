import { app } from 'electron';
import path from 'node:path';
import { connectMcpHost } from './mcp-client.mjs';
import { startMcpStdio } from './mcp-stdio.mjs';
import { EXTERNAL_MCP_INSTRUCTIONS } from './external-spec';

/** Fixed packaged-app entrypoint for external clients; no editor is launched. */
export async function runExternalMcpEntry(): Promise<void> {
  app.setName('Powermove');
  if (process.env.POWERMOVE_USER_DATA) {
    if (!path.isAbsolute(process.env.POWERMOVE_USER_DATA)) throw new Error('POWERMOVE_USER_DATA must be an absolute profile path.');
    app.setPath('userData', process.env.POWERMOVE_USER_DATA);
  }
  if (process.platform === 'darwin') app.setActivationPolicy('prohibited');
  /* This relay exists for one open Powermove. When the app quits, exit too
     instead of idling in the background; the client reconnects to a fresh
     relay after Powermove is opened again. The pause lets the stdio layer
     report any interrupted call before the process ends. */
  const client = await connectMcpHost(app.getPath('userData'), {
    onHostClosed: () => { setTimeout(() => app.exit(0), 250); }
  });
  await startMcpStdio({ ...client, instructions: EXTERNAL_MCP_INSTRUCTIONS });
  app.exit(0);
}
