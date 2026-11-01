import { spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import type { LegacyKeychain } from './secrets';
/** Configuration is included in the signed bundle, never renderer supplied.
 * Missing config is expected on the review branch: cutover remains blocked. */
export async function signedKeychain(resources: string): Promise<{ keychain?: LegacyKeychain; designatedRequirements?: readonly [string, string] }> {
  if (process.platform !== 'darwin') return {};
  let config: { applications: [string, string]; designatedRequirements: [string, string] };
  try { config = JSON.parse(await readFile(path.join(resources, 'transition', 'identities.json'), 'utf8')); }
  catch (error: any) { if (error.code === 'ENOENT') return {}; throw new Error('Invalid signed transition identities.'); }
  if (!Array.isArray(config.applications) || config.applications.length !== 2 || config.applications.some(p => typeof p !== 'string' || !path.isAbsolute(p))
    || !Array.isArray(config.designatedRequirements) || config.designatedRequirements.length !== 2) throw new Error('Invalid signed transition identities.');
  const keychain: LegacyKeychain = {
    putLegacyItem(input) {
      return new Promise((resolve, reject) => {
        const child = spawn(path.join(resources, 'transition', 'powermove-transition-keychain'), [], { stdio: 'pipe' });
        let output = '';
        const fail = () => { child.kill(); reject(new Error('Keychain handoff failed.')); };
        const timer = setTimeout(fail, 120_000);
        child.once('error', () => { clearTimeout(timer); fail(); });
        child.stdin.on('error', () => { /* exit/error supplies the sanitized failure */ });
        child.stdout.on('data', chunk => { output += String(chunk); if (output.length > 64 * 1024) fail(); });
        child.stderr.resume(); // Never forward a helper's diagnostics or secret material.
        child.once('exit', code => {
          clearTimeout(timer);
          if (code !== 0) { reject(new Error('Keychain handoff failed.')); return; }
          try { resolve(JSON.parse(output)); } catch { reject(new Error('Invalid Keychain receipt.')); }
        });
        child.stdin.end(JSON.stringify({ ...input, value: Buffer.from(input.value).toString('base64'), applications: config.applications }));
      });
    }
  };
  return { keychain, designatedRequirements: config.designatedRequirements };
}
