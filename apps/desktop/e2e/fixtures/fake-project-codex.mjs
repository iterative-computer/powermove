#!/usr/bin/env node
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { serveCodexFixture } from './codex-app-server.mjs';

await serveCodexFixture(async (_thread, turn) => {
  const id = process.env.FAKE_CODEX_EXTENSION_ID;
  if (id) {
    const prompt = turn.input.find(item => item.type === 'text')?.text ?? '';
    const stage = prompt.match(/Stage only in (.+?);/)?.[1];
    if (!stage || !/^[a-z0-9-]+$/.test(id)) throw new Error('Missing isolated extension stage');
    const directory = path.join(stage, id);
    await mkdir(directory, { recursive: true });
    await writeFile(path.join(directory, 'manifest.json'), JSON.stringify({ id, name: 'Fake extension', version: '1.0.0', apiVersion: 3, permissions: [], entry: 'index.ts', author: 'agent' }));
    await writeFile(path.join(directory, 'index.ts'), 'export default function activate() {}\n');
  }
  return process.env.FAKE_CODEX_RESULT ? JSON.parse(process.env.FAKE_CODEX_RESULT)
    : { summary: 'done', commands: [], artifacts: [], externalActions: [], notes: [], extensions: [] };
});
