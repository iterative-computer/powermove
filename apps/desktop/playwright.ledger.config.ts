import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';

import { defineConfig } from '@playwright/test';

import base from './playwright.config';

/**
 * The e2e ledger run (`bun run test:e2e:ledger`): the normal suite minus the
 * specs quarantined in e2e/quarantine.json. Every entry is explained in
 * docs/engine/e2e-ledger.md with its error, reason, and owning lane.
 * `bun run test:e2e` keeps running everything; only this config skips.
 */

type QuarantineEntry = {
  spec: string;
  /** Exact test titles; omit to quarantine the whole spec file. */
  tests?: string[];
  status: 'fail' | 'flaky' | 'env';
  owner: string;
  reason: string;
};

const STATUSES = new Set(['fail', 'flaky', 'env']);
// Plan section 27 lanes.
const LANES = new Set([
  'Model and evaluation',
  'Renderer write path',
  'Text and vectors',
  '3D',
  'Media',
  'GPU and render',
  'Color',
  'Engine and platform',
  'Platform, CI, and compliance',
  'Audio',
  'Export and agents',
  'Proxies and storage',
  'Viewer and settings UI',
  'Parity and QA'
]);

const testDir = path.resolve(__dirname, 'e2e');
const quarantineFile = path.join(testDir, 'quarantine.json');

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function loadQuarantine(): QuarantineEntry[] {
  const parsed = JSON.parse(readFileSync(quarantineFile, 'utf8')) as { entries?: unknown };
  if (!Array.isArray(parsed.entries)) throw new Error(`${quarantineFile}: "entries" must be an array`);
  const seen = new Set<string>();
  return parsed.entries.map((raw, index) => {
    const entry = raw as QuarantineEntry;
    const where = `${quarantineFile} entries[${index}]`;
    if (typeof entry.spec !== 'string' || !entry.spec.endsWith('.spec.ts')) throw new Error(`${where}: "spec" must name a .spec.ts file`);
    if (!existsSync(path.join(testDir, entry.spec))) throw new Error(`${where}: ${entry.spec} does not exist; remove the stale entry`);
    if (!STATUSES.has(entry.status)) throw new Error(`${where}: "status" must be fail, flaky, or env`);
    if (!LANES.has(entry.owner)) throw new Error(`${where}: "owner" must be a plan section 27 lane, got ${JSON.stringify(entry.owner)}`);
    if (typeof entry.reason !== 'string' || !entry.reason.trim()) throw new Error(`${where}: "reason" is required`);
    if (entry.tests !== undefined) {
      if (!Array.isArray(entry.tests) || entry.tests.length === 0 || entry.tests.some(title => typeof title !== 'string' || !title)) {
        throw new Error(`${where}: "tests" must be a non-empty list of test titles, or omitted for the whole file`);
      }
    }
    const key = `${entry.spec}\u0000${(entry.tests ?? ['*']).join('\u0000')}`;
    if (seen.has(key)) throw new Error(`${where}: duplicate entry for ${entry.spec}`);
    seen.add(key);
    return entry;
  });
}

const entries = loadQuarantine();
const wholeFiles = entries.filter(entry => !entry.tests).map(entry => entry.spec);
// Playwright matches grep against "<file path> <describe titles> <test title>
// <tags>", where "@tag" words in titles are repeated as trailing tags.
// Anchoring on the file name keeps a title from matching in another spec, and
// anchoring the title on a word boundary keeps "x" from matching "a x".
const titles = entries.flatMap(entry =>
  (entry.tests ?? []).map(title => new RegExp(
    `(^|[/\\\\ ])${escapeRegExp(entry.spec)} (.* )?${escapeRegExp(title)}( @\\S+)*$`
  ))
);

export default defineConfig({
  ...base,
  // The ledger run is the whole suite. With one worker it took 38 minutes on
  // the reference machine (docs/engine/e2e-ledger.md), so the base config's
  // 10-minute global timeout would stop it a quarter of the way through.
  globalTimeout: 2 * 60 * 60_000,
  testIgnore: wholeFiles.map(spec => new RegExp(`[/\\\\]${escapeRegExp(spec)}$`)),
  ...(titles.length ? { grepInvert: titles } : {})
});
