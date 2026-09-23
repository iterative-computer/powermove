import { expect, test } from 'bun:test';
import { scanFiles, scanText } from '../src/scan';
/* Fixtures are assembled at runtime so no provider-shaped literal exists in
   the source: GitHub push protection and our own publish scanner would
   otherwise flag this test file. */
const samples = {
  openai_key: 'sk-' + 'ABCDEFGHIJKLMNOPQRSTUVWXYZ012345',
  anthropic_key: 'sk-ant-' + 'ABCDEFGHIJKLMNOPQRSTUVWXYZ012345',
  aws_access_key: 'AKIA' + 'ABCDEFGHIJKLMNOP',
  github_token: 'ghp_' + 'ABCDEFGHIJKLMNOPQRSTUVWXYZ012345',
  gitlab_token: 'glpat-' + 'ABCDEFGHIJKLMNOPQRSTUVWXYZ012345',
  slack_token: 'xoxb-' + '1234567890-1234567890-abcdefghijkl',
  stripe_key: 'sk_live_' + 'ABCDEFGHIJKLMNOPQRSTUVWXYZ012345',
  google_api_key: 'AIza' + 'A'.repeat(35),
  jwt: ['eyJhbGciOiJIUzI1NiJ9', 'eyJzdWIiOiIxMjM0NTY3ODkwIn0', 'SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c'].join('.'),
  pem_private_key: '-----BEGIN ' + 'RSA PRIVATE KEY-----'
} as const;
for (const [kind, token] of Object.entries(samples)) test(kind, () => expect(scanText('index.ts', `const value = '${token}'`).some((f) => f.kind === kind && f.hard)).toBe(true));
test('entropy waiver and hard override', () => {
  const entropy = 'q7Az9M2bP4xT8nV6kR3wY5cH1jL0sD4fG9uQ';
  const result = scanFiles([{ path: 'index.ts', text: `// powermove-secret-ok: test fixture\nconst v = '${entropy}';\nconst key = '${samples.github_token}'; // powermove-secret-ok: test fixture` }]);
  expect(result.waived.some((f) => f.kind === 'high_entropy' && f.waived === 'test fixture')).toBe(true);
  expect(result.blocked.some((f) => f.kind === 'github_token' && f.hard)).toBe(true);
});
test('nonsecret content and binary skipped', () => {
  expect(scanText('manifest.json', '{"id":"my-extension","version":"1.2.3"}')).toEqual([]);
  expect(scanText('index.ts', 'const png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAUAABAAAA"')).toEqual([]);
  expect(scanText('style.css', '.card { content: "flex grow-0 shrink-0 rounded-md border-neutral-200 bg-slate-100 text-center"; }')).toEqual([]);
  expect(scanText('image.png', samples.aws_access_key)).toEqual([]);
  expect(scanText('index.ts', `\0${samples.aws_access_key}`)).toEqual([]);
});

test('inline CSS declaration lists are not credentials', () => {
  const css = "el.style.cssText='position:absolute;right:124px;top:7px;z-index:6;display:flex;align-items:center';";
  expect(scanText('viewer.ts', css)).toEqual([]);
});
