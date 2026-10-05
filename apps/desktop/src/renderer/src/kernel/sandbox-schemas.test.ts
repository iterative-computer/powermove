import { expect, it } from 'vitest';
import { registrationSchemas } from './sandbox-schemas';

const base = { id: 'x.fx', label: 'FX', group: 'Stylize', params: [], frag: 'void main(){}' };

it('accepts every documented EffectDefinition field in the sandbox', () => {
  // These are all optional fields on the public EffectDefinition (kernel/api.ts);
  // the sandbox registration schema is .strict(), so any it omits would reject a
  // valid effect with unrecognized_keys (regression: `backdrop` did exactly that).
  const parsed = registrationSchemas.effects.safeParse({
    ...base, passes: 2, keepOrig: true, backdrop: true, rawShader: true, viewportSafe: true, viewportPadding: ['radius'], ui: [{kind:'point',x:'x',y:'y'}]
  });
  expect(parsed.success).toBe(true);
});

it('still rejects a genuinely unknown effect key', () => {
  const parsed = registrationSchemas.effects.safeParse({ ...base, notAField: true });
  expect(parsed.success).toBe(false);
});

it('accepts a theme with rootAttributes (host strips it for sandboxed themes)', () => {
  // sandboxTheme() neutralizes rootAttributes; the schema must accept the field
  // so registration reaches that strip instead of failing first.
  const parsed = registrationSchemas.theme.safeParse({ id: 'x.t', name: 'T', scheme: 'auto', tokens: { '--accent': '#f60' }, rootAttributes: { 'data-win98': '' } });
  expect(parsed.success).toBe(true);
});

// Guardrail: these omissions are DELIBERATE sandbox restrictions, not drift.
// A sandboxed binding is forced to priority >= 1000 and may not fire in fields,
// and sandboxed (iframe) panels don't get host-chrome controls. Do not "fix"
// these by widening the schema — that would weaken the sandbox.
it('keeps rejecting capability fields the sandbox intentionally withholds', () => {
  expect(registrationSchemas.keybindings.safeParse({ key: 'cmd+k', command: 'x.run', inFields: true }).success).toBe(false);
  expect(registrationSchemas.keybindings.safeParse({ key: 'cmd+k', command: 'x.run', priority: -1 }).success).toBe(false);
  expect(registrationSchemas.panels.safeParse({ id: 'x.p', title: 'P', headless: true }).success).toBe(false);
});
