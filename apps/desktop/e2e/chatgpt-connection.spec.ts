import path from 'node:path';
import { readFile, stat } from 'node:fs/promises';

import { expect, launchApp, repoRoot, test } from './helpers/app';

test('Settings reads a ChatGPT subscription through the real main-process bridge', async () => {
  const session = await launchApp({
    env: {
      CODEX_BINARY: path.join(repoRoot, 'src/main/codex/__fixtures__/fake-codex-app-server.sh')
    }
  });
  try {
    await session.openEditor(); await session.openAgent();
    const { page } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Agent connection' }) }));
      PM.ProjectsScreen.hide(); PM.SpatialAssistant.open();
    });
    await page.getByRole('button', { name: 'Open settings', exact: true }).click();
    const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
    await expect(settings).toBeVisible();
    const generalTab = settings.getByRole('button', { name: 'General', exact: true });
    const accountsTab = settings.getByRole('button', { name: 'Accounts', exact: true });
    const extensionsTab = settings.getByRole('button', { name: 'Extensions', exact: true });
    await generalTab.click();
    await expect(generalTab).toHaveAttribute('aria-current', 'location');
    await expect(settings.getByRole('region', { name: 'Accounts', exact: true })).toBeVisible();
    await expect(settings.getByRole('region', { name: 'Extensions', exact: true })).toBeVisible();
    await accountsTab.click();
    await expect(accountsTab).toHaveAttribute('aria-current', 'location');
    await expect(settings.getByRole('region', { name: 'Accounts', exact: true })).toBeVisible();
    await expect(settings).toContainText('ChatGPT');
    await expect(settings).toContainText('motion@example.com · Pro plan');
    await extensionsTab.click();
    await expect(extensionsTab).toHaveAttribute('aria-current', 'location');
    const extensions = settings.getByRole('list', { name: 'Extensions' });
    await expect(settings).toContainText('Manage extensions');
    await expect(extensions).toBeVisible();
    await accountsTab.click();
    const disconnect = settings.getByRole('button', { name: 'Disconnect', exact: true });
    await expect(disconnect).toBeEnabled();
    await disconnect.click();
    await expect(settings.getByRole('button', { name: 'Connect', exact: true }).first()).toBeEnabled();
    await expect(settings).toContainText('Use your ChatGPT subscription with Powermove.');
    await expect(page.locator('[data-agent-panel] .agent-connect-gate')).toContainText('Connect ChatGPT');
    await expect(page.locator('[data-agent-panel] [aria-label="Message composer"]')).toHaveCount(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});

test('Connect opens the trusted ChatGPT browser flow and enters waiting state', async () => {
  const session = await launchApp({
    env: {
      CODEX_BINARY: path.join(repoRoot, 'src/main/codex/__fixtures__/fake-codex-app-server.sh'),
      POWERMOVE_FAKE_CHATGPT_STATUS: 'disconnected'
    }
  });
  try {
    await session.openEditor(); await session.openAgent();
    await session.app.evaluate(({ shell }) => {
      (globalThis as any).__powermoveOpenedAuthUrl = null;
      shell.openExternal = async (url: string) => {
        (globalThis as any).__powermoveOpenedAuthUrl = url;
      };
    });
    const { page } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Agent connection' }) }));
      PM.ProjectsScreen.hide(); PM.SpatialAssistant.open();
    });
    const gate = page.locator('[data-agent-panel] .agent-connect-gate');
    await expect(gate).toContainText('Use your ChatGPT subscription to power the Powermove agent.');
    const connect = gate.getByRole('button', { name: 'Connect ChatGPT', exact: true });
    await expect(connect).toBeEnabled();
    await connect.click();
    await expect(gate.getByRole('button', { name: 'Waiting for sign-in…', exact: true })).toBeDisabled();
    await expect(gate).toContainText('Finish signing in in your browser.');
    await expect.poll(() => session.app.evaluate(() =>
      (globalThis as any).__powermoveOpenedAuthUrl
    )).toBe('https://chatgpt.com/auth/powermove-e2e');
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});

test('large images and files cross the real renderer-to-main agent boundary', async () => {
  const session = await launchApp({
    env: {
      CODEX_BINARY: path.join(repoRoot, 'src/main/codex/__fixtures__/fake-codex.sh')
    }
  });
  try {
    await session.openEditor();
    const result = await session.page.evaluate(async () => {
      const image = new Uint8Array(4 * 1024 * 1024 + 1);
      image.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
      let binary = '';
      for (let offset = 0; offset < image.length; offset += 0x8000) {
        binary += String.fromCharCode(...image.subarray(offset, offset + 0x8000));
      }
      // Exercise the same shim conversion and private IPC used by the agent.
      return await (window as any).PM.CodexBridge.request('Use the attached references.', null, [window.btoa(binary)], {
        mode: 'autonomous',
        model: null,
        reasoningEffort: null,
        access: 'project',
        projectId: 'large-attachment-project',
        projectName: 'Large attachment project',
        projectJSON: '{"layers":[]}',
        attachments: [{ name: 'large.bin', content: '\0'.repeat(100 * 1024 + 1) }],
      });
    });

    expect(JSON.parse(result.text)).toMatchObject({ summary: 'done' });
    const inputs = path.join(session.userData, 'Agent Workspaces', 'large-attachment-project', 'inputs');
    const reference = await readFile(path.join(inputs, 'references', 'reference-0.png'));
    expect(reference.byteLength).toBe(4 * 1024 * 1024 + 1);
    expect([...reference.subarray(0, 8)]).toEqual([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
    // The workspace sanitizes attachment filenames (large.bin → large-bin).
    expect((await stat(path.join(inputs, 'attachments', 'large-bin'))).size).toBe(100 * 1024 + 1);
    expect(await session.page.evaluate(() => typeof (window as any).powermove)).toBe('undefined');
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});

test('Claude subscription status and structured runs cross the real hidden app boundary', async () => {
  const session = await launchApp({
    env: {
      CODEX_BINARY: path.join(repoRoot, 'src/main/codex/__fixtures__/fake-codex-app-server.sh'),
      CLAUDE_BINARY: path.join(repoRoot, 'src/main/claude/__fixtures__/fake-claude.sh')
    }
  });
  try {
    await session.openEditor(); await session.openAgent();
    const { page } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Agent connection' }) }));
      PM.ProjectsScreen.hide(); PM.SpatialAssistant.open();
    });
    await page.getByRole('button', { name: 'Open settings', exact: true }).click();
    const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
    await expect(settings).toContainText('Claude');
    await expect(settings).toContainText('claude@example.com · Max plan');
    await settings.getByRole('main').getByRole('button', { name: 'Done', exact: true }).click();

    await session.openAgent();

    const provider = page.locator('.agent-modelbar select[aria-label="Provider"]');
    await provider.selectOption('claude');
    await expect(page.locator('.agent-modelbar select[aria-label="Model"]')).toHaveValue('sonnet');
    await expect(page.locator('[data-agent-panel] [aria-label="Message composer"]')).toBeVisible();

    const result = await page.evaluate(async () => await (window as any).PM.CodexBridge.request(
      'Return a message.',
      { type: 'object', required: ['message'], properties: { message: { type: 'string' } } },
      [], {
        provider: 'claude',
        mode: 'editor',
        model: 'sonnet',
        reasoningEffort: 'high',
        access: 'editor',
        projectId: 'editor',
        projectName: 'Editor',
        projectJSON: null,
        attachments: [],
      }
    ));
    expect(JSON.parse(result)).toEqual({ message: 'claude editor done' });
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});
