import { lstat, readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { scanFiles, type ScanFinding, type ScanKind } from '@powermove/registry/scan';
import type { AgentExtensionChange } from '../../shared/ipc';
import { EXTENSION_ID, MANIFEST_LIMITS, parseManifest } from '../../shared/extensions';
import { compileExtension } from '../extensions/compiler';
import type { AgentWorkspace } from './workspace';
import { AgentResultValidationError } from './result-repair';

/** Check generated source with the same compiler used by the live mod loader. */
export async function validateStagedExtensions(
  layout: Pick<AgentWorkspace, 'stagingDirectory' | 'runDirectory'>,
  changes: readonly AgentExtensionChange[]
): Promise<void> {
  for (const change of changes) {
    if (!EXTENSION_ID.test(change.id)) throw new Error('Invalid staged extension id.');
    if (change.action === 'removed') continue;
    const dir = path.join(layout.stagingDirectory, change.id);
    let raw: unknown;
    try { raw = JSON.parse(await readFile(path.join(dir, 'manifest.json'), 'utf8')); }
    catch (error) {
      if (!(error instanceof SyntaxError) && (error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
      throw new AgentResultValidationError(`${change.id}: provide a valid manifest.json in the current staging directory. Nothing was published.`);
    }
    const manifest = parseManifest(raw);
    if (!manifest.ok) throw new AgentResultValidationError(`${change.id}: ${manifest.error}. Nothing was published.`);
    if (manifest.manifest.id !== change.id) throw new AgentResultValidationError(`${change.id}: the manifest id must match its folder. Nothing was published.`);
    // Credentials never reach the library: values are declared and read at runtime.
    const { blocked } = scanFiles(await readStagedText(dir));
    if (blocked.length) throw new AgentResultValidationError(blocked.map((finding) => credentialLine(change.id, finding)).join('\n'));
    const compiled = await compileExtension({ dir, entry: manifest.manifest.entry || 'index.ts', outDir: path.join(layout.runDirectory, '.compiled') });
    if (!compiled.ok) throw new AgentResultValidationError(`${change.id} failed compilation: ${compiled.error}. Repair its staged source. Nothing was published.`);
  }
}

const KIND_LABEL: Record<ScanKind, string> = {
  openai_key: 'an OpenAI API key',
  anthropic_key: 'an Anthropic API key',
  aws_access_key: 'an AWS access key',
  github_token: 'a GitHub token',
  gitlab_token: 'a GitLab token',
  slack_token: 'a Slack token',
  stripe_key: 'a Stripe key',
  google_api_key: 'a Google API key',
  jwt: 'a JWT',
  pem_private_key: 'a private key',
  high_entropy: 'a secret'
};

function credentialLine(id: string, finding: ScanFinding): string {
  return `${id}/${finding.path}:${finding.line} looks like ${KIND_LABEL[finding.kind]}. Declare it in manifest vars and read api.vars. Nothing was published.`;
}

/* The file types `scanText` reads (packages/registry/src/scan.ts); binaries
   and anything else are skipped without being read. */
const SCANNED = /(?:\.(?:ts|js|mjs|svelte|json|md|txt|css|html|frag|vert|glsl|wgsl|yml|yaml|toml)|(?:^|\/)\.env[^/]*)$/i;
const SCAN_FILE_BYTES = 2 * 1024 * 1024;

async function readStagedText(root: string): Promise<Array<{ path: string; text: string }>> {
  const files: Array<{ path: string; text: string }> = [];
  let visited = 0;
  async function walk(directory: string, prefix: string): Promise<void> {
    const entries = await readdir(directory, { withFileTypes: true });
    entries.sort((a, b) => a.name.localeCompare(b.name));
    for (const entry of entries) {
      if (entry.name === 'node_modules' || entry.name === '.git') continue;
      if (++visited > MANIFEST_LIMITS.sourceFiles) return;
      const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
      const full = path.join(directory, entry.name);
      const metadata = await lstat(full);
      if (metadata.isSymbolicLink()) continue;
      if (metadata.isDirectory()) { await walk(full, relative); continue; }
      if (!metadata.isFile() || metadata.size > SCAN_FILE_BYTES || !SCANNED.test(relative)) continue;
      const bytes = await readFile(full);
      if (bytes.subarray(0, 8192).includes(0)) continue;
      files.push({ path: relative, text: bytes.toString('utf8') });
    }
  }
  await walk(root, '');
  return files;
}
