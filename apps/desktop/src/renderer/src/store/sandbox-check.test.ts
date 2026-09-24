// @vitest-environment happy-dom
import { expect, it } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import SandboxCheckStatus from './SandboxCheckStatus.svelte';
import { sandboxCheckLines, type SandboxCheckReport } from './sandbox-check';

const report = (patch: Partial<SandboxCheckReport>): SandboxCheckReport => ({
  ok: false, activation: 'ok', permissionErrors: [], cspViolations: [], asyncMisuse: [],
  panels: [], runtimeErrors: [], durationMs: 1, ...patch
});

it('renders each blocked reason as a separate row', async () => {
  const target = document.createElement('div');
  document.body.append(target);
  const blocked = report({
    permissionErrors: [{ namespace: 'render', member: 'gl.bounds', count: 1 }],
    panels: [{ id: 'ease-lab', mounted: false, error: 'panel boom' }]
  });
  const component = mount(SandboxCheckStatus, { target, props: { state: { status: 'done', report: blocked } } });
  flushSync();
  const rows = [...target.querySelectorAll('[data-sandbox="problem"]')].map(row => row.textContent?.trim());
  expect(rows).toEqual(sandboxCheckLines(blocked));
  expect(rows[0]).toContain('api.render.gl.bounds');
  expect(rows[1]).toContain("Panel 'ease-lab' failed to mount: panel boom");
  await unmount(component);
  target.remove();
});

it('renders the success line', async () => {
  const target = document.createElement('div');
  document.body.append(target);
  const component = mount(SandboxCheckStatus, { target, props: { state: { status: 'done', report: report({ ok: true }) } } });
  flushSync();
  expect(target.querySelector('[data-sandbox="ok"]')?.textContent).toContain('Runs in the sandbox');
  expect(target.querySelector('[data-sandbox="problem"]')).toBeNull();
  await unmount(component);
  target.remove();
});
