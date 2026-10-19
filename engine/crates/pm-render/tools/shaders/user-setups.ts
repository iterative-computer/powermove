/** Real-loader capture for the private shader corpus. Run under pm-electron-lock.
 * Sources, projects, diagnostics and names are written only outside Git.
 * The missing-directory notice is deliberate: CI has no private user data.
 */
import { cp, mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { homedir, tmpdir } from 'node:os';
import { join, resolve, sep } from 'node:path';
const REPO = process.env['PM_SHADER_GATE_REPO'] ?? resolve(import.meta.dirname, '../../../../..');
import { launchApp } from '../../../../../apps/desktop/e2e/helpers/app';
import { decodeProjectContainer } from '../../../../../apps/desktop/src/shared/project-container';

// Playwright's Electron inspector handshake uses Node's WebSocket stack.
// Bun's handshake can hang even though Electron boots. Transpile this tool
// for Node, preserving the app helper's source directory and external deps.
if (process.versions.bun) {
  const file = join(REPO, 'apps/desktop/.vite/shader-user-setups.mjs');
  const built = await Bun.build({ entrypoints: [import.meta.path], target: 'node', format: 'esm', packages: 'external', define: { __dirname: JSON.stringify(join(REPO, 'apps/desktop/e2e/helpers')) } });
  if (!built.success) throw new Error(built.logs.join('\n'));
  await mkdir(resolve(file, '..'), { recursive: true });
  await writeFile(file, await built.outputs[0]!.text());
  const child = Bun.spawn(['node', '--max-old-space-size=8192', file, ...process.argv.slice(2)], { stdout: 'inherit', stderr: 'inherit', env: { ...process.env, PM_SHADER_GATE_REPO: REPO } });
  process.exit(await child.exited);
}

const args = process.argv.slice(2);
const flag = (key: string) => args.includes(key) ? args[args.indexOf(key) + 1] : undefined;
const root = resolve(flag('--root') ?? join(homedir(), 'Library/Application Support/Powermove Engine Corpus/user-setups'));
const out = resolve(flag('--out') ?? join(root, 'shader-loader.json'));
if (out.startsWith(resolve(REPO) + sep)) throw new Error('private loader capture must stay outside the repository');
const captures: any[] = [];
if (!existsSync(root)) {
  console.log('SKIP user-setups: private corpus directory is absent');
} else for (const folder of (await readdir(root, { withFileTypes: true })).filter(d => d.isDirectory()).sort((a,b) => a.name.localeCompare(b.name))) {
  const setup = join(root, folder.name);
  if (!existsSync(join(setup, 'extensions'))) { console.log('SKIP user setup without extension sources'); continue; }
  const profile = await mkdtemp(join(tmpdir(), 'pm-shader-user-'));
  try {
    await cp(join(setup, 'extensions'), join(profile, 'extensions'), { recursive: true });
    const expected = (await readdir(join(profile, 'extensions'))).filter(id => existsSync(join(profile, 'extensions', id, 'manifest.json')));
    const projects: any[] = [];
    if (existsSync(join(setup, 'projects'))) for (const name of (await readdir(join(setup, 'projects'))).filter(n => n.endsWith('.pmv')).sort()) {
      const decoded: any = decodeProjectContainer(new Uint8Array(await readFile(join(setup, 'projects', name)))).document;
      projects.push(decoded.proj ?? decoded);
      if (!Array.isArray(projects.at(-1)?.layers)) throw new Error('user project has no composition layers');
    }
    const modes: any[] = [];
    for (const mode of ['0', '1']) {
      const disk = execFileSync('df', ['-k', '/System/Volumes/Data'], { encoding: 'utf8' }).trim().split('\n').at(-1)!.trim().split(/\s+/);
      if (Number(disk[3]) < 40 * 1024 * 1024) throw new Error('disk guard: less than 40 GiB free; retry the capture after freeing this lane target');
      const modeProfile = join(profile, `mode-${mode}`);
      await cp(join(profile, 'extensions'), join(modeProfile, 'extensions'), { recursive: true });
      console.log(`user setup: launching engine setting ${mode}`);
      const session = await launchApp({ userData: modeProfile, env: { POWERMOVE_NEW_ENGINE: mode } });
      try {
        console.log('user setup: editor boot');
        await session.openEditor();
        await session.page.waitForFunction(count => (window as any).PM.Kernel.loader.records().filter((r: any) => r.scope === 'user').length >= count, expected.length, { timeout: 45_000 });
        console.log('user setup: waiting for extension activation');
        await session.page.evaluate(async () => { await Promise.race([(window as any).PM.Kernel.loader.whenIdle(), new Promise((_, reject) => setTimeout(() => reject(new Error('user extension loader did not settle within 45 s')), 45_000))]); });
        console.log('user setup: collecting registrations and evaluated project channels');
        // Deep protocol serialization expands dense animation channels into
        // millions of tagged objects. Send exact JSON in bounded chunks.
        await session.page.evaluate(() => { (window as any).__shaderUserProjects = []; });
        for (const project of projects) {
          const json = JSON.stringify(project);
          await session.page.evaluate(() => { (window as any).__shaderUserChunks = []; });
          for (let i = 0; i < json.length; i += 1 << 20) await session.page.evaluate(chunk => { (window as any).__shaderUserChunks.push(chunk); }, json.slice(i, i + (1 << 20)));
          await session.page.evaluate(() => { const w = window as any; w.__shaderUserProjects.push(JSON.parse(w.__shaderUserChunks.join(''))); delete w.__shaderUserChunks; });
        }
        const captured = await session.page.evaluate(async ({ expected }) => {
          const projects = (window as any).__shaderUserProjects;
          const PM = (window as any).PM, K = PM.Kernel;
          const registrations: any[] = [], signatures = new Set<string>();
          const captureRegistrations = () => {
            for (const owner of expected) for (const [kind, registry] of [['effect', K.effects], ['transition', K.transitions], ['layer', K.layerTypes]]) {
              for (const entry of registry.ownerEntries(owner)) {
                const item = JSON.parse(JSON.stringify({ owner, kind, def: entry.item, winning: registry.topEntry(entry.id) === entry }));
                const signature = JSON.stringify(item);
                if (!signatures.has(signature)) { signatures.add(signature); registrations.push(item); }
              }
            }
          };
          captureRegistrations();
          const uses: any[] = [];
          for (let pi = 0; pi < projects.length; pi++) {
            PM.proj = projects[pi]; PM.bus.emit('project');
            await Promise.race([K.loader.whenIdle(), new Promise((_, reject) => setTimeout(() => reject(new Error('project extension reload did not settle within 45 s')), 45_000))]);
            await new Promise(resolve => setTimeout(resolve, 0));
            captureRegistrations();
            const visit = (layers: any[], comp: string) => {
              for (const layer of layers ?? []) {
                for (const fx of layer.fx ?? []) {
                  const entry = K.effects.topEntry(fx.type);
                  if (!entry || !expected.includes(entry.ownerId)) continue;
                  const times = new Set<number>([Math.max(0, layer.from), Math.max(0, layer.from + layer.dur / 2)]);
                  for (const prop of Object.values(fx.p ?? {}) as any[]) for (const key of prop?.kf ?? []) {
                    const time = Number(key.t); if (Number.isFinite(time)) times.add(time);
                  }
                  const sorted = [...times].sort((a,b) => a-b);
                  // Include interpolation inside every keyframe segment.
                  sorted.slice(1).forEach((t,i) => times.add((t + sorted[i]!) / 2));
                  for (const time of [...times].sort((a,b) => a-b)) {
                    const params = entry.item.params.map((param: any) => {
                      const prop = fx.p?.[param.k];
                      return prop ? (PM.evP(layer, prop, time, param.k) ?? param.def) : param.def;
                    });
                    uses.push({ kind: 'effect', owner: entry.ownerId, id: fx.type, registration: registrations.findIndex(r => r.owner === entry.ownerId && r.kind === 'effect' && JSON.stringify(r.def) === JSON.stringify(entry.item)), project: pi, comp, layer: layer.id, time, params });
                  }
                }
              }
            };
            visit(PM.proj.layers, PM.proj.compId);
            for (const [id, comp] of Object.entries(PM.proj.comps ?? {}) as any) visit(comp.layers, id);
          }
          return JSON.parse(JSON.stringify({ registrations, uses, health: K.loader.records().filter((r: any) => expected.includes(r.id)).map((r: any) => ({ id: r.id, health: r.health })) }));
        }, { expected });
        for (const row of captured.health) if (row.health.error) row.health.error = row.health.error.replaceAll(modeProfile, '<isolated-profile>');
        modes.push({ mode, ...captured });
        console.log(`user setup: captured ${captured.registrations.length} registrations and ${captured.uses.length} project samples`);
      } finally {
        // This profile is disposable; never autosave a large private
        // project merely to shut down a collector window.
        await session.app.evaluate(({ app }) => { app.exit(0); }).catch(() => undefined);
        await session.close();
      }
    }
    captures.push({ setup: folder.name, expected, modes });
    console.log(`user setup: ${expected.length} extensions; ${modes[0].registrations.length}/${modes[1].registrations.length} registrations off/on`);
  } finally { await rm(profile, { recursive: true, force: true }); }
}
await mkdir(resolve(out, '..'), { recursive: true });
await writeFile(out, JSON.stringify({ format: 'powermove-user-shader-loader', captures }, null, 2));
