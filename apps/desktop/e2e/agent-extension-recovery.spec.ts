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

    await session.openEditor();
    const projectId = await session.page.evaluate(() => (window as any).PM.proj.id);
    await session.page.evaluate(() => {
      const PM = (window as any).PM;
      const nativeRequest = PM.CodexBridge.request;
      let requests = 0;
      // Exercise the real staging/promotion path once. The follow-up review
      // makes no additional changes; returning the same creation twice is not
      // a faithful model response to automatic verification.
      PM.CodexBridge.request = (...args: any[]) => requests++ === 0 ? nativeRequest(...args)
        : Promise.resolve({ text: JSON.stringify({ summary: 'Verified the loaded extension.', commands: [], artifacts: [], externalActions: [], notes: [] }), extensions: [] });
      PM.AgentUI.setAccess('project');
      PM.AgentUI.submit('Create the requested test extension.');
    });
    await session.page.waitForFunction(() => (window as any).PM.AgentUI.state.phase === 'result');
    const result = await session.page.evaluate(() => {
      const run = (window as any).PM.AgentUI.state.run;
      return { extensions: run.extensions, extensionChangeSetId: run.undoRuns.find((entry: any) => entry.extensionChangeSetId)?.extensionChangeSetId, reviewError: run.reviewError };
    });
    expect(result).toMatchObject({
      extensions: [{ id: extensionId, action: 'created' }],
      extensionChangeSetId: expect.any(String), reviewError: '',
    });
    if (!result.extensionChangeSetId) throw new Error('Agent run did not create a recoverable change set.');

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

    await session.page.evaluate(() => (window as any).PM.AgentUI.undoSceneRun());
    expect(await session.page.evaluate(() => (window as any).PM.AgentUI.state.conversation.at(-1)?.text)).toContain('restored');
    expect(await exists(liveExtension)).toBe(false);
    expect(await exists(path.join(path.dirname(recordPath), 'redo', extensionId, 'manifest.json'))).toBe(true);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});
