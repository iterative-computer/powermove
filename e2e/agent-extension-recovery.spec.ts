import { access, readFile } from 'node:fs/promises';
import path from 'node:path';

import { expect, launchApp, repoRoot, test } from './helpers/app';

async function exists(file: string): Promise<boolean> {
  try {
    await access(file);
    return true;
  } catch {
    return false;
  }
}

test('agent extension changes are isolated, recorded, and recoverable through the app bridge', async () => {
  const extensionId = 'e2e-isolated-extension';
  const projectId = 'e2e-isolated-project';
  const session = await launchApp({
    env: {
      CODEX_BINARY: path.join(repoRoot, 'src/main/codex/__fixtures__/fake-codex.sh'),
      FAKE_CODEX_EXTENSION_ID: extensionId,
      FAKE_CODEX_RESULT: JSON.stringify({
        summary: 'Created an isolated extension.',
        commands: [],
        artifacts: [],
        externalActions: [],
        notes: [],
        extensions: [{ id: extensionId, action: 'created', summary: 'E2E recovery fixture' }]
      })
    }
  });

  try {
    const liveExtension = path.join(session.userData, 'extensions', extensionId);
    expect(await exists(liveExtension)).toBe(false);

    const result = await session.page.evaluate(async ({ projectId }) =>
      await (window as any).powermove.codex.run({
        id: 'e2e-isolated-extension-run',
        mode: 'autonomous',
        prompt: 'Create the requested test extension.',
        schema: null,
        images: [],
        model: null,
        reasoningEffort: null,
        access: 'project',
        projectId,
        projectName: 'E2E isolated project',
        projectJSON: '{"layers":[]}',
        attachments: [],
        consentToken: null
      }), { projectId });

    expect(result).toMatchObject({
      ok: true,
      extensions: [{ id: extensionId, action: 'created' }],
      extensionChangeSetId: expect.any(String)
    });
    if (!result.ok || !result.extensionChangeSetId) throw new Error('Agent run did not create a recoverable change set.');

    expect(await exists(path.join(liveExtension, 'manifest.json'))).toBe(true);
    const recordPath = path.join(
      session.userData,
      'Agent Change History',
      projectId,
      result.extensionChangeSetId,
      'change-set.json'
    );
    const record = JSON.parse(await readFile(recordPath, 'utf8'));
    expect(record).toMatchObject({
      id: result.extensionChangeSetId,
      projectId,
      changes: [{ id: extensionId, action: 'created' }]
    });

    const restored = await session.page.evaluate(async ({ projectId, changeSetId }) =>
      await (window as any).powermove.codex.restoreChangeSet({ projectId, changeSetId }), {
        projectId,
        changeSetId: result.extensionChangeSetId
      });

    expect(restored).toMatchObject({
      changeSetId: result.extensionChangeSetId,
      extensions: [{ id: extensionId, action: 'created' }]
    });
    expect(await exists(liveExtension)).toBe(false);
    expect(await exists(path.join(path.dirname(recordPath), 'redo', extensionId, 'manifest.json'))).toBe(true);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});
