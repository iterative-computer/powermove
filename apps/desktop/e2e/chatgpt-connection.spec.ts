import path from 'node:path';

import { expect, launchApp, repoRoot, test } from './helpers/app';

test('Settings reads a ChatGPT subscription through the real main-process bridge', async () => {
  const session = await launchApp({
    env: {
      CODEX_BINARY: path.join(repoRoot, 'src/main/codex/__fixtures__/fake-codex-app-server.sh')
    }
  });
  try {
    await session.openEditor();
    const { page } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Agent connection' }) }));
      PM.ProjectsScreen.hide(); PM.SpatialAssistant.open();
    });
    await page.getByRole('button', { name: 'Open settings', exact: true }).click();
    const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
    await expect(settings).toBeVisible();
    const generalTab = settings.getByRole('tab', { name: 'General', exact: true });
    const accountsTab = settings.getByRole('tab', { name: 'Accounts', exact: true });
    const extensionsTab = settings.getByRole('tab', { name: 'Extensions', exact: true });
    await expect(generalTab).toHaveAttribute('aria-selected', 'true');
    await expect(settings.getByRole('tabpanel', { name: 'Accounts', exact: true })).toBeHidden();
    await expect(settings.getByRole('tabpanel', { name: 'Extensions', exact: true })).toBeHidden();
    await accountsTab.click();
    await expect(accountsTab).toHaveAttribute('aria-selected', 'true');
    await expect(settings.getByRole('tabpanel', { name: 'Accounts', exact: true })).toBeVisible();
    await expect(settings).toContainText('ChatGPT');
    await expect(settings).toContainText('motion@example.com · Pro plan');
    await extensionsTab.click();
    await expect(extensionsTab).toHaveAttribute('aria-selected', 'true');
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
    await session.openEditor();
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
    const result = await session.page.evaluate(async () => {
      const image = new Uint8Array(4 * 1024 * 1024 + 1);
      image.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
      return await (window as any).powermove.codex.run({
        id: 'large-attachment-run',
        mode: 'autonomous',
        prompt: 'Use the attached references.',
        schema: null,
        images: [image],
        model: null,
        reasoningEffort: null,
        access: 'project',
        projectId: 'large-attachment-project',
        projectName: 'Large attachment project',
        projectJSON: '{"layers":[]}',
        attachments: [{ name: 'large.bin', data: new Uint8Array(100 * 1024 + 1) }],
        consentToken: null
      });
    });

    expect(result).toMatchObject({ ok: true });
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
    await session.openEditor();
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
    await settings.getByRole('button', { name: 'Done', exact: true }).click();

    const provider = page.locator('.agent-modelbar select[aria-label="Provider"]');
    await provider.selectOption('claude');
    await expect(page.locator('.agent-modelbar select[aria-label="Model"]')).toHaveValue('sonnet');
    await expect(page.locator('[data-agent-panel] [aria-label="Message composer"]')).toBeVisible();

    const result = await page.evaluate(async () => await (window as any).powermove.codex.run({
      id: 'claude-hidden-run',
      provider: 'claude',
      mode: 'editor',
      prompt: 'Return a message.',
      schema: { type: 'object', required: ['message'], properties: { message: { type: 'string' } } },
      images: [],
      model: 'sonnet',
      reasoningEffort: 'high',
      access: 'editor',
      projectId: 'editor',
      projectName: 'Editor',
      projectJSON: null,
      attachments: [],
      consentToken: null
    }));
    expect(result).toMatchObject({ ok: true, text: '{"message":"claude editor done"}' });
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
  }
});
