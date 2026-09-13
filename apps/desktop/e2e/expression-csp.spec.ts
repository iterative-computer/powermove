import { expect, test } from './helpers/app';

test('evaluates project expressions without unsafe-eval in the editor document', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.proj = PM.mkProject({ name: 'Expressions', w: 64, h: 64, fps: 30, dur: 2 });
    const first = PM.mkLayer('solid', { name: 'First' });
    const second = PM.mkLayer('solid', { name: 'Second' });
    PM.proj.layers = [first, second];
    second.p['position.x'].v = 10;
    second.p['position.x'].expr = 'value + Math.sin(t) * 10 + idx + param("gain")';
    PM.proj.params.gain = { value: 3 };
    const response = await fetch(window.location.href);
    return {
      value: PM.ev(second, 'position.x', 0.5),
      policy: response.headers.get('content-security-policy') || '',
    };
  });

  expect(result.value).toBeCloseTo(18.794255, 5);
  expect(result.policy).toContain("script-src 'self'");
  expect(result.policy).not.toContain("'unsafe-eval'");
  expect(session.diagnostics.pageErrors).toEqual([]);
});
