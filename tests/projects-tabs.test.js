const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
function utf8(rel) { return fs.readFileSync(path.join(root, rel), 'utf8'); }

/* ── registry harness: in-memory store ─────────────────── */
function projectsModel() {
  const mem = new Map();
  const PM = {
    uid: (() => { let n = 0; return p => `${p}${++n}`; })(),
    store: {
      get(k, d) { return mem.has(k) ? mem.get(k) : d; },
      set(k, v) { mem.set(k, v); },
      del(k) { mem.delete(k); },
    },
    serialize() { return JSON.stringify(PM.proj); },
    version: '1.0.0',
    proj: null,
  };
  const context = vm.createContext({ window: { PM }, console });
  vm.runInContext(utf8('js/core/projects.js'), context);
  return { PM, mem };
}

test('registry round-trips project data and metadata', () => {
  const { PM } = projectsModel();
  const proj = { id: 'P1', name: 'Hero', layers: [{ id: 'La' }] };
  PM.proj = proj;
  PM.Projects.put(proj);
  assert.deepEqual([...PM.Projects.list().map(m => m.name)], ['Hero']);
  const back = PM.Projects.get('P1');
  assert.equal(back.name, 'Hero');
  assert.deepEqual([...back.layers.map(l => l.id)], ['La']);
});

test('tab-owned editor state round-trips separately for each project', () => {
  const { PM } = projectsModel();
  PM.Projects.putState('P1', { time: 1, workspace: { id: 'edit-one' }, detached: ['assets'] });
  PM.Projects.putState('P2', { time: 7, workspace: { id: 'edit-two' }, detached: ['inspector'] });
  assert.deepEqual(PM.Projects.getState('P1'), { time: 1, workspace: { id: 'edit-one' }, detached: ['assets'] });
  assert.deepEqual(PM.Projects.getState('P2'), { time: 7, workspace: { id: 'edit-two' }, detached: ['inspector'] });
});

test('registry unwraps real save envelopes and boot picks the first project with content', () => {
  const { PM } = projectsModel();
  PM.store.set('project.empty', JSON.stringify({ v: '1.0.0', proj: { id: 'empty', name: 'Untitled', layers: [] } }));
  PM.store.set('project.hero', JSON.stringify({ v: '1.0.0', proj: { id: 'hero', name: 'Hero', layers: [{ id: 'L1' }] } }));
  PM.Projects.saveList([{ id: 'empty', name: 'Untitled', at: 2 }, { id: 'hero', name: 'Hero', at: 1 }]);
  PM.store.set('openTabs', ['empty']);
  assert.equal(PM.Projects.get('hero').layers.length, 1);
  assert.equal(PM.Projects.pickBoot().id, 'hero');
});

test('boot ignores anonymous empty projects and accepts a named empty project as fallback', () => {
  const { PM } = projectsModel();
  const get = id => ({ id, name: id === 'named' ? 'Storyboard' : 'Untitled', layers: [] });
  assert.equal(PM.Projects.pickBoot({ tabs: ['blank'], metas: [{ id: 'blank' }], get }), null);
  assert.equal(PM.Projects.pickBoot({ tabs: ['blank'], metas: [{ id: 'blank' }, { id: 'named' }], get }).id, 'named');
});

test('upsertMeta changes metadata without overwriting stored data', () => {
  const { PM } = projectsModel();
  PM.proj = { id: 'P1', name: 'Hero', layers: [] };
  PM.Projects.put(PM.proj);
  /* a rename must never write the CURRENT composition into another slot */
  PM.proj = { id: 'P9', name: 'Other', layers: [{ id: 'Lz' }] };
  PM.Projects.upsertMeta({ id: 'P1', name: 'Hero renamed', at: 42 });
  assert.equal(PM.Projects.get('P1').name, 'Hero', 'stored data untouched');
  const meta = PM.Projects.list().find(m => m.id === 'P1');
  assert.equal(meta.name, 'Hero renamed');
});

test('rename updates the project slot, registry metadata, and active project together', () => {
  const { PM } = projectsModel();
  PM.proj = { id: 'P1', name: 'Hero', layers: [{ id: 'L1' }] };
  PM.Projects.put(PM.proj);
  assert.equal(PM.Projects.rename('P1', '  Velocity Study  '), 'Velocity Study');
  assert.equal(PM.proj.name, 'Velocity Study');
  assert.equal(PM.Projects.get('P1').name, 'Velocity Study');
  assert.equal(PM.Projects.list().find(m => m.id === 'P1').name, 'Velocity Study');
  assert.equal(PM.Projects.rename('P1', '   '), 'Velocity Study', 'a blank rename keeps the current name');
});

test('remove clears both metadata and the storage slot', () => {
  const { PM, mem } = projectsModel();
  PM.proj = { id: 'P1', name: 'X', layers: [] };
  PM.Projects.put(PM.proj);
  PM.Projects.remove('P1');
  assert.equal(PM.Projects.list().length, 0);
  assert.equal(mem.has('project.P1'), false);
  assert.equal(PM.Projects.get('P1'), null);
});

test('trash is recoverable until explicitly destroyed', () => {
  const { PM, mem } = projectsModel();
  PM.proj = { id: 'P1', name: 'Recover me', layers: [{ id: 'L1' }] };
  PM.Projects.put(PM.proj);
  assert.equal(PM.Projects.trash('P1'), true);
  assert.equal(PM.Projects.list().length, 0);
  assert.equal(PM.Projects.trashList()[0].name, 'Recover me');
  assert.equal(mem.has('project.P1'), true, 'trash keeps project data');
  assert.equal(PM.Projects.restore('P1'), true);
  assert.equal(PM.Projects.get('P1').layers.length, 1);
  PM.Projects.trash('P1'); PM.Projects.destroy('P1');
  assert.equal(mem.has('project.P1'), false, 'delete forever removes data');
});

test('open-tab bookkeeping keeps visible order stable when an existing tab is selected', () => {
  const { PM } = projectsModel();
  PM.proj = { id: 'A', name: 'A', layers: [] };
  PM.Projects.put(PM.proj);
  PM.proj = { id: 'B', name: 'B', layers: [] };
  PM.Projects.put(PM.proj);
  PM.Projects.markOpen('A');
  PM.Projects.markOpen('B');
  PM.Projects.markOpen('A');
  assert.deepEqual([...PM.Projects.tabs()], ['A', 'B'], 'selection does not reorder tabs');
  PM.Projects.markClosed('A');
  assert.deepEqual([...PM.Projects.tabs()], ['B']);
  PM.Projects.markOpen('A');
  assert.deepEqual([...PM.Projects.tabs()], ['B', 'A'], 'explicitly reopening a closed tab appends it');
  PM.Projects.markClosed('A');
  /* a tab pointing at a deleted project disappears instead of crashing boot */
  PM.Projects.markOpen('GONE');
  assert.deepEqual([...PM.Projects.tabs()], ['B']);
});

test('get() falls back to the legacy autosave slot for a matching id', () => {
  const { PM } = projectsModel();
  PM.store.set('autosave', { v: 1, at: 0, proj: { id: 'Pold', name: 'Legacy', layers: [] } });
  const p = PM.Projects.get('Pold');
  assert.ok(p && p.name === 'Legacy');
  assert.equal(PM.Projects.get('other'), null);
});

/* ── layout & wiring pins ──────────────────────────────── */
test('normalization narrowly strips retired Layers and Generative panels from saved workspaces', () => {
  let n = 0;
  const PM = { uid: p => `${p}${++n}`, clamp: (v, a, b) => Math.max(a, Math.min(b, v)), h: () => ({}) };
  vm.runInContext(utf8('js/core/workspace.js'), vm.createContext({ window: { PM }, console }));
  const w = PM.WS.normalize({
    id: 'w1', name: 'Old',
    layout: { docks: [
      { id: 'left', panels: [{ id: 'layers' }, { id: 'library' }, { id: 'assets' }] },
      { id: 'center', panels: [{ id: 'viewer', flex: true }] },
    ] },
  });
  const ids = w.layout.docks.flatMap(d => d.panels.map(q => q.id));
  assert.equal(ids.includes('layers'), false, 'Layers panel no longer docks');
  assert.equal(ids.includes('library'), false, 'retired Generative panel no longer docks');
  assert.ok(ids.includes('assets'));
});

test('tabs, projects screen, and drag preview are wired end to end', () => {
  const app = utf8('js/app.js');
  const index = utf8('index.html');
  const layout = utf8('js/ui/layout.js');
  const shortcuts = utf8('js/ui/shortcuts.js');
  assert.match(app, /function openTab\(id\)/, 'tab switching exists');
  assert.match(app, /function closeTab\(id\)/, 'tab closing exists');
  assert.match(app, /pm-open-project/, 'projects screen opens projects through the app bridge');
  assert.match(app, /ProjectsScreen\.isOpen\) PM\.ProjectsScreen\.hide/, 'an active tab exits the Projects screen');
  assert.doesNotMatch(index, /class="brand"/, 'top chrome uses project controls, not a decorative brand block');
  assert.match(layout, /panel-drop-preview/, 'drag shows a live placement preview');
  assert.match(layout, /drag-src/, 'the dragged source is dimmed during preview');
  assert.match(shortcuts, /Projects screen/, '⌘P opens the projects screen');
  assert.match(app, /function captureProjectSession\(\)/);
  assert.match(app, /workspace: PM\.WS\.snapshot\(\), time: PM\.time/);
  assert.match(app, /selection: \{ layers: \[\.\.\.PM\.sel\.layers\], keys: PM\.sel\.keys\.filter\(key => typeof key === 'string'\)/,
    'project sessions persist keyframe ids only');
  assert.match(app, /timeline: \{ pps: PM\.TL\.pps, scrollT: PM\.TL\.scrollT, scrollY: PM\.TL\.scrollY, graph: PM\.TL\.graph \}/);
  assert.match(app, /detached: PM\.Popout\?\.openIds/);
  assert.match(app, /PM\.LibraryUI\?\.close\?\.\(\)/);
  assert.match(app, /PM\.SpatialAssistant\?\.cancel\?\.\(\)/);
  assert.match(app, /PM\.Popout\?\.closeAll\?\.\(\)/);
  assert.match(app, /session\?\.workspace\) PM\.WS\.restoreSnapshot/);
  assert.match(app, /tab\.ondblclick[\s\S]*beginProjectRename/, 'double-clicking a project tab starts inline rename');
  assert.match(app, /e\.detail > 1[\s\S]*beginProjectRename/,
    'the second click also starts rename across WebKit double-click variations');
  assert.match(app, /e\.key === 'Enter'[\s\S]*finish\(true\)/, 'Enter commits the inline name');
  assert.match(app, /e\.key === 'Escape'[\s\S]*finish\(false\)/, 'Escape cancels the inline name');
  assert.match(app, /input\.onblur = \(\) => finish\(true\)/, 'clicking away commits the inline name');
  assert.match(app, /if \(tabs\.querySelector\('\.project-doc\.renaming'\)\) return/,
    'autosave refreshes cannot tear down a live rename field');
  assert.ok(index.indexOf('js/core/projects.js') > -1 && index.indexOf('js/ui/projects.js') > -1,
    'registry and screen are loaded');
});

test('projects surface has production library controls and Powermove selection cues', () => {
  const app = utf8('js/app.js');
  const screen = utf8('js/ui/projects.js');
  const css = utf8('css/app.css');
  assert.match(screen, /Search projects/);
  assert.match(screen, /All Projects/);
  assert.match(screen, /Recently edited/);
  assert.match(screen, /Move to Trash/);
  assert.match(screen, /Delete Forever/);
  assert.match(css, /\.ps-navbtn\.on\{background:var\(--accent-dim\);color:var\(--accent-tx\)\}/,
    'active project navigation uses Powermove orange');
  assert.doesNotMatch(css, /project-doc-state/,
    'project tabs do not show an orange dot beside the project name');
  assert.doesNotMatch(app, /h\('i\.project-doc-state'/);
  assert.match(app, /'aria-label': tabName \+ \(dirty \? ', unsaved' : ''\)/,
    'unsaved status remains meaningful to assistive technology');
  assert.match(css, /#tabs\{[\s\S]*background:var\(--bg-sunken\)/,
    'project tabs live in one intentional compact switcher');
  assert.match(css, /#tabs\{[\s\S]*border:0/,
    'project switcher has no outer border');
  assert.match(css, /#tabs\{[\s\S]*width:max-content[\s\S]*flex:0 1 auto/,
    'the switcher wraps its projects instead of leaving an empty trough');
  assert.match(css, /\.project-doc-input\{[^}]*min-width:0[^}]*width:100%/,
    'the inline rename field stays inside the compact tab');
  assert.match(screen, /className = 'ps-grid'[\s\S]*' empty'/,
    'empty project sections receive a full-height centered layout');
  assert.match(css, /\.ps-grid\.empty\{[^}]*place-items:center/);
});

test('project controls use defined outline icons instead of fallback dots', () => {
  const util = utf8('js/core/util.js');
  const screen = utf8('js/ui/projects.js');
  ['project', 'search', 'list', 'trash', 'more'].forEach(name => {
    assert.match(util, new RegExp('\\n\\s*' + name + ":\\s*'"), `${name} icon is defined`);
    assert.match(screen, new RegExp("'" + name + "'"), `${name} icon is used`);
  });
  const css = utf8('css/app.css');
  assert.match(css, /\.ps-search svg,[\s\S]*stroke:currentColor;fill:none/,
    'project icons receive the shared outline rendering style');
  assert.match(css, /\.btn svg\{width:13px;height:13px[\s\S]*stroke:currentColor/,
    'button icons cannot expand and push their labels outside');
});

test('every app icon has a global rendering contract and a registry entry', () => {
  const util = utf8('js/core/util.js');
  const css = utf8('css/app.css');
  const iconBlock = util.slice(util.indexOf('PM.ICONS = {'), util.indexOf('\n};', util.indexOf('PM.ICONS = {')));
  const defined = new Set([...iconBlock.matchAll(/^\s*([A-Za-z][A-Za-z0-9]*):/gm)].map(m => m[1]));
  const requested = new Set();
  for (const file of fs.readdirSync(path.join(root, 'js'), { recursive: true })) {
    if (typeof file !== 'string' || !file.endsWith('.js')) continue;
    const src = utf8(path.join('js', file));
    for (const match of src.matchAll(/PM\.icon\(['"]([^'"]+)['"]\)/g)) requested.add(match[1]);
  }
  assert.deepEqual([...requested].filter(name => !defined.has(name)), [], 'literal icon requests are all defined');
  ['solid', 'text', 'shape', 'image', 'video', 'audio', 'shader', 'null', 'precomp'].forEach(type => {
    const model = utf8('js/core/model.js');
    const icon = model.match(new RegExp(type + ":\\s*\\{\\s*icon:\\s*'([^']+)'"));
    assert.ok(icon && defined.has(icon[1]), `${type} model icon is defined`);
  });
  assert.match(util, /classList\.add\('pm-icon'\)/, 'every generated icon receives the shared class');
  assert.match(util, /dataset\.iconSet = 'phosphor'/, 'every catalog icon identifies the Phosphor source');
  assert.match(css, /\.pm-icon\{[^}]*fill:currentColor;stroke:none/, 'Phosphor fill geometry is rendered globally');
  assert.match(util, /Unknown Powermove icon/, 'unknown dynamic names are diagnosed instead of silently becoming dots');
  assert.doesNotMatch(iconBlock, /undefined|<svg/, 'the registry contains valid Phosphor path fragments');
  const inspector = utf8('js/ui/inspector.js');
  const assistant = utf8('js/assistant/spatial.js');
  assert.doesNotMatch(inspector, /PM\.svg\(/, 'inspector one-off drawings use catalog icons');
  assert.match(assistant, /PM\.icon\('return'\)/, 'assistant send actions use the compact Phosphor Return-key icon');
  assert.doesNotMatch(assistant, />\s*['"](?:↵|↑|×)['"]/, 'assistant actions do not use text glyphs as icons');
});

test('development packages cannot write into the installed app storage', () => {
  const build = utf8('scripts/build-macos-app.sh');
  assert.match(build, /com\.zellzoi\.powermove\.dev/, 'ordinary builds use isolated WebKit storage');
  assert.match(build, /--release[\s\S]*com\.zellzoi\.powermove/, 'release mode keeps the shipping identity');
});

test('autosave debounces and unload cancels the same timer', () => {
  const app = utf8('js/app.js');
  assert.match(app, /clearTimeout\(APP\.saveTimer\);\s*APP\.saveTimer = setTimeout/, 'one APP-owned debounce timer');
  assert.match(app, /beforeunload[\s\S]*clearTimeout\(APP\.saveTimer\)/, 'unload cancels that timer');
});
