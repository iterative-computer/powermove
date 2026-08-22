const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const root = path.resolve(__dirname, '..');
const read = file => fs.readFileSync(path.join(root, file), 'utf8');
const util = read('js/core/util.js');
const layout = read('js/ui/layout.js');
const timeline = read('js/ui/timeline.js');
const inspector = read('js/ui/inspector.js');
const projects = read('js/ui/projects.js');
const shortcuts = read('js/ui/shortcuts.js');
const library = read('js/ui/library.js');

test('menu infrastructure closes after working commands and blocks unavailable commands', () => {
  assert.match(util, /aria-disabled.*!!it\.disabled/s);
  assert.match(util, /if \(it\.disabled\) return; PM\.closeMenus\(\); it\.run && it\.run\(\)/);
  assert.match(util, /if \(!el\.contains\(event\.target\)\) PM\.closeMenus\(\)/,
    'pointer-down inside the menu cannot remove a command before its click runs');
  assert.match(util, /removeEventListener\('pointerdown', PM\._menuOutside\)/,
    'outside-click cleanup cannot leak after selection or cancellation');
});

test('panel options expose only reachable movement, hide, detach, and restoration commands', () => {
  assert.match(layout, /button\.panel-options/);
  assert.match(layout, /Pop out to window.*PM\.Popout\.open/s);
  assert.match(layout, /Hide panel.*hidePanel/s);
  assert.match(layout, /disabled: dock\.id === 'left'/);
  assert.match(layout, /disabled: dock\.id === 'center'/);
  assert.match(layout, /disabled: dock\.id === 'right'/);
  assert.match(layout, /Restore \$\{PM\.PANELS\[item\.id\]\.title\}/);
  assert.doesNotMatch(layout, /label: 'Hide panel'.*spec\.id === 'viewer'/,
    'Composition is filtered before a destructive menu command can be created');
});

test('timeline context commands target the right-clicked layer and match registered shortcuts', () => {
  assert.match(timeline, /if \(!PM\.sel\.layers\.includes\(L\.id\)\) PM\.selectLayers\(L\.id\)/);
  assert.match(timeline, /Split at playhead'.*kb: '⌘⇧D'.*disabled: !inside/s);
  assert.match(timeline, /Duplicate'.*kb: '⌘D'.*PM\.cmd\('duplicate'\)/s);
  assert.match(timeline, /Precompose'.*kb: '⌘⇧C'.*PM\.cmd\('precompose'\)/s);
  assert.match(shortcuts, /def\('duplicate', 'Duplicate layers', '⌘D'/);
  assert.match(shortcuts, /def\('split', 'Split at playhead', '⌘⇧D'/);
  assert.match(shortcuts, /def\('precompose', 'Precompose selected layers…', '⌘⇧C'/);
});

test('context-sensitive menus disable safe no-ops and retain unique actions', () => {
  assert.match(inspector, /disabled: !p\.kf\.length/);
  assert.match(projects, /label: 'Open', disabled: m\.id === PM\.proj\.id/);
  assert.match(library, /disabled = workspace\.id === PM\.WS\.current\.id/);
  assert.match(library, /Edit validated definition….*PM\.WS\.editJSON/s);
  assert.doesNotMatch(read('js/ui/panels.js'), /registerPanel\('library'/);
});

test('every product right-click surface prevents the browser menu and routes through Powermove menus', () => {
  for (const [name, source] of [
    ['panels', layout], ['timeline', timeline], ['inspector', inspector], ['projects', projects],
  ]) {
    assert.match(source, /contextmenu/ , `${name} has a product context-menu surface`);
    assert.match(source, /preventDefault\(\)/, `${name} suppresses the browser menu`);
    assert.match(source, /PM\.menu\(/, `${name} routes visible commands through the common menu lifecycle`);
  }
});
