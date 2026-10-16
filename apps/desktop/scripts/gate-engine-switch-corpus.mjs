/**
 * Plan section 21's engine-switch gate over the parity corpus, for the Phase 2
 * gate (docs/engine/phase2-gate.md): "every corpus project without
 * native-only features round-trips flag on → off → on with no loss".
 *
 * One profile, four hidden launches of the built app:
 *
 *   1. Use New Engine on: every synthetic project of the newest corpus v1
 *      capture is opened the way the corpus capture opens it (the
 *      generated document through `pm-open-project`, its `media:` files
 *      imported through `PM.assets.add`, references rewritten to the
 *      assigned asset ids, reopened) and stored (`PM.Projects.put`).
 *   2. On again: each project opened from the project list; the stored
 *      project as the new engine holds it is the baseline (opening fills
 *      defaults), with every asset loaded.
 *   3. Off: each opened, compared with the baseline (keys sorted; open and
 *      save timestamps aside), every asset loaded, stored again.
 *   4. On: each opened and compared again, every asset loaded.
 *
 * Projects stamped `requiresEngine: "native"` would be refused while off;
 * the corpus has none (it was captured by the legacy engine), and the
 * refusal itself is e2e/media-store.spec.ts's case.
 *
 *   bun run build && bun run build:engine     (in apps/desktop)
 *   ~/Library/Caches/powermove-engine/bin/pm-electron-lock node scripts/gate-engine-switch-corpus.mjs [--corpus <capture dir>] [--only <prefix>] [--json out.json]
 *
 * Hidden, never focused windows (POWERMOVE_BACKGROUND_TEST=1), a temporary
 * profile removed afterwards. Exit status 1 when any project loses anything.
 */
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { _electron } from 'playwright';

const desktop = path.resolve(import.meta.dirname, '..');
const repo = path.resolve(desktop, '../..');
const arg = (name) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : undefined;
};
const root = path.join(os.homedir(), 'Library/Application Support/Powermove Engine Corpus');
const corpus = arg('corpus') ?? path.join(root, readdirSync(root).filter((d) => d.startsWith('v1-')).sort().at(-1));
const only = arg('only');
const index = JSON.parse(readFileSync(path.join(corpus, 'generated/index.json'), 'utf8'));
const fixtures = path.join(desktop, 'e2e/fixtures');

function mediaRefs(value, out = new Set()) {
  if (typeof value === 'string') { if (value.startsWith('media:')) out.add(value.slice(6)); }
  else if (Array.isArray(value)) value.forEach((v) => mediaRefs(v, out));
  else if (value && typeof value === 'object') Object.values(value).forEach((v) => mediaRefs(v, out));
  return out;
}
const resolveMedia = (name) => [path.join(fixtures, name), path.join(corpus, 'generated/media', name)].find((f) => existsSync(f));

const profile = mkdtempSync(path.join(os.tmpdir(), 'pm-gate-switch-'));

async function launch(flag) {
  const env = { ...process.env, POWERMOVE_USER_DATA: profile, POWERMOVE_DEVTOOLS: '0', POWERMOVE_BACKGROUND_TEST: '1', POWERMOVE_NEW_ENGINE: flag ? '1' : '0', POWERMOVE_FAKE_CHATGPT_STATUS: 'connected', CODEX_BINARY: path.join(desktop, 'src/main/codex/__fixtures__/fake-codex-app-server.sh') };
  delete env.ELECTRON_RUN_AS_NODE;
  const app = await _electron.launch({ args: ['.'], cwd: desktop, env });
  await app.evaluate(({ dialog }) => {
    dialog.showSaveDialog = async () => ({ canceled: true, filePath: '' });
    dialog.showOpenDialog = async () => ({ canceled: true, filePaths: [] });
    dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false });
  });
  const page = await app.firstWindow();
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e.stack || e.message)));
  await page.waitForURL((url) => url.protocol === 'app:' || url.hostname === 'localhost');
  await page.waitForFunction(() => Boolean(window.PM?.ProjectsScreen && window.PM?.mkProject && window.PM?.openProjectHere), null, { timeout: 120_000 });
  await page.evaluate(async () => { await window.PM.Kernel.loader.builtinsReady; });
  return { app, page, errors };
}

async function close({ app }) {
  await app.evaluate(({ dialog }) => { dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false }); }).catch(() => {});
  await app.close().catch(() => {});
}

/** The project as stored, keys sorted, open and save timestamps aside (as e2e/native-engine-tracer.spec.ts). */
const stored = (page, id) => page.evaluate((id) => {
  const PM = window.PM;
  const project = PM.proj?.id === id ? PM.proj : PM.Projects.get(id);
  const sort = (value) => {
    if (Array.isArray(value)) return value.map(sort);
    if (value && typeof value === 'object') {
      const out = {};
      for (const key of Object.keys(value).sort()) {
        if (['opened', 'updated', 'modified', 'savedAt'].includes(key)) continue;
        if (value[key] !== undefined) out[key] = sort(value[key]);
      }
      return out;
    }
    return value;
  };
  return JSON.stringify(sort(JSON.parse(JSON.stringify(project))));
}, id);

/** Opens a stored project; waits for its assets; lists assets that did not load. */
const openStored = (page, id) => page.evaluate(async (id) => {
  const PM = window.PM;
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  if (PM.proj?.id !== id) {
    const listed = PM.Projects.list().some((meta) => meta.id === id);
    const refused = !!PM.refusedByEngine?.(id);
    const ok = await PM.openProjectHere(id);
    if (!ok) return { opened: false, refused: true, why: !listed ? 'not in the project list' : refused ? 'refused: requires the new engine' : 'openProjectHere returned false', missing: [] };
  }
  PM.ProjectsScreen?.hide?.();
  await PM.projectAssetsReady;
  PM.pause?.();
  const deadline = performance.now() + 30_000;
  const missing = () => Object.keys(PM.proj.assets || {}).filter((a) => !PM.assets.get(a) || PM.assets.errors?.has?.(a));
  while (missing().length && performance.now() < deadline) await sleep(100);
  return { opened: PM.proj.id === id, refused: false, missing: missing().map((a) => `${a} (${PM.proj.assets[a]?.name ?? '?'}): ${PM.assets.errors?.get?.(a) ?? 'not loaded'}`), requiresEngine: PM.proj.requiresEngine ?? null };
}, id);

const projects = index.projects.filter((p) => !only || p.id.startsWith(only));
const results = Object.fromEntries(projects.map((p) => [p.id, { id: p.id, media: p.media, problems: [] }]));
const pageErrors = [];
const compared = {};
const t0 = Date.now();
try {
  // 1. On: open, import media, store.
  let s = await launch(true);
  try {
    await s.page.evaluate(() => {
      const input = document.createElement('input');
      input.type = 'file'; input.multiple = true; input.id = 'gate-media';
      input.style.cssText = 'position:fixed;width:1px;height:1px;opacity:0;pointer-events:none;left:-10px;top:-10px';
      document.body.appendChild(input);
    });
    for (const p of projects) {
      const r = results[p.id];
      const doc = JSON.parse(readFileSync(path.join(corpus, 'generated/projects', p.id, 'document.json'), 'utf8'));
      const names = [...mediaRefs(doc)];
      const files = names.map(resolveMedia);
      if (files.some((f) => !f)) { r.skipped = `media not found: ${names.filter((_, i) => !files[i]).join(', ')}`; continue; }
      r.docId = doc.id;
      try {
        await s.page.evaluate(async (doc) => {
          const PM = window.PM;
          window.dispatchEvent(new CustomEvent('pm-open-project', { detail: doc }));
          PM.ProjectsScreen?.hide?.();
          for (let i = 0; i < 400 && PM.proj?.id !== doc.id; i++) await new Promise((res) => setTimeout(res, 25));
          if (PM.proj?.id !== doc.id) throw new Error(`project ${doc.id} did not open`);
          await PM.projectAssetsReady;
        }, doc);
        if (files.length) {
          await s.page.setInputFiles('#gate-media', files);
          await s.page.evaluate(async (names) => {
            const PM = window.PM;
            const input = document.getElementById('gate-media');
            const map = {};
            for (const file of Array.from(input.files || [])) map[file.name] = (await PM.assets.add(file, { newAsset: true, silent: true })).id;
            input.value = '';
            const missing = names.filter((n) => !map[n]);
            if (missing.length) throw new Error(`not imported: ${missing.join(', ')}`);
            const visit = (v) => {
              if (typeof v === 'string' && v.startsWith('media:')) return map[v.slice(6)] ?? v;
              if (Array.isArray(v)) return v.map(visit);
              if (v && typeof v === 'object') { for (const k of Object.keys(v)) v[k] = visit(v[k]); return v; }
              return v;
            };
            const project = JSON.parse(JSON.stringify(PM.proj));
            project.layers = visit(project.layers);
            project.comps = visit(project.comps);
            window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
            for (let i = 0; i < 400 && PM.proj?.layers !== undefined && JSON.stringify(PM.proj.layers).includes('media:'); i++) await new Promise((res) => setTimeout(res, 25));
            await PM.projectAssetsReady;
          }, names);
        }
        await s.page.evaluate(async () => { await window.PM.Projects.put(window.PM.proj); });
        r.stored = true;
      } catch (e) { r.problems.push(`on (import): ${e.message.split('\n')[0]}`); }
    }
    // Let media copies into the store settle and the store reach main before quitting.
    await s.page.waitForTimeout(3000);
    const listed = await s.page.evaluate(async () => { await window.PM.store.flush?.(); return window.PM.Projects.list().map((m) => m.id); });
    for (const p of projects) if (results[p.id].stored && !listed.includes(results[p.id].docId)) results[p.id].problems.push('on (import): not in the project list after storing');
    pageErrors.push(...s.errors.map((e) => `on (import): ${e.split('\n')[0]}`));
  } finally { await close(s); }

  const phases = [['on (baseline)', true], ['off', false], ['on again', true]];
  for (const [phase, flag] of phases) {
    s = await launch(flag);
    try {
      for (const p of projects) {
        const r = results[p.id];
        if (!r.stored) continue;
        try {
          const o = await openStored(s.page, r.docId);
          if (!o.opened) { r.problems.push(`${phase}: ${o.why ?? 'did not open'}`); continue; }
          if (o.missing.length) r.problems.push(`${phase}: assets not loaded: ${o.missing.join('; ')}`);
          if (o.requiresEngine) r.requiresEngine = o.requiresEngine;
          const now = await stored(s.page, r.docId);
          compared[phase] = (compared[phase] ?? 0) + 1;
          r.assets = Math.max(r.assets ?? 0, (await s.page.evaluate(() => Object.keys(window.PM.proj.assets || {}).length)));
          if (phase === 'on (baseline)') r.baseline = now;
          else if (now !== r.baseline) {
            const a = JSON.parse(r.baseline), b = JSON.parse(now);
            const diff = Object.keys({ ...a, ...b }).filter((k) => JSON.stringify(a[k]) !== JSON.stringify(b[k]));
            r.problems.push(`${phase}: stored project differs in ${diff.join(', ')}`);
          }
          if (!flag) await s.page.evaluate(async () => { await window.PM.Projects.put(window.PM.proj); await window.PM.store.flush?.(); });
        } catch (e) { r.problems.push(`${phase}: ${e.message.split('\n')[0]}`); }
      }
      pageErrors.push(...s.errors.map((e) => `${phase}: ${e.split('\n')[0]}`));
    } finally { await close(s); }
  }
} finally {
  rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}

const list = Object.values(results);
const summary = {
  corpus: path.basename(corpus), projects: list.length,
  roundTripped: list.filter((r) => r.stored && !r.problems.length).length,
  withMedia: list.filter((r) => r.stored && r.media.length).length,
  assets: list.reduce((n, r) => n + (r.assets ?? 0), 0),
  compared,
  failed: list.filter((r) => r.problems.length).map((r) => ({ id: r.id, problems: r.problems })),
  skipped: list.filter((r) => r.skipped).map((r) => ({ id: r.id, reason: r.skipped })),
  stamped: list.filter((r) => r.requiresEngine).map((r) => r.id),
  pageErrors,
  seconds: Math.round((Date.now() - t0) / 1000),
};
process.stdout.write(`${summary.roundTripped} of ${summary.projects} corpus projects round-trip on → off → on with no loss (${summary.withMedia} with media, ${summary.assets} assets; compared ${JSON.stringify(compared)}); ${summary.failed.length} failed, ${summary.skipped.length} skipped, ${summary.stamped.length} stamped; ${summary.seconds} s\n`);
for (const f of summary.failed) process.stdout.write(`  ${f.id}: ${f.problems.join(' | ')}\n`);
for (const k of summary.skipped) process.stdout.write(`  skipped ${k.id}: ${k.reason}\n`);
for (const e of pageErrors) process.stdout.write(`  page error ${e}\n`);
const json = arg('json');
if (json) writeFileSync(json, JSON.stringify(summary, null, 2) + '\n');
process.exit(summary.failed.length ? 1 : 0);
