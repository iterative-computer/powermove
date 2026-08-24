const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const root = path.join(__dirname, '..');
const panels = fs.readFileSync(path.join(root, 'js/ui/panels.js'), 'utf8');
const css = fs.readFileSync(path.join(root, 'css/app.css'), 'utf8');

test('Project media has one compact Import action, useful metadata, and explicit timeline actions', () => {
  assert.match(panels, /registerPanel\('assets', \{\s*title: 'Media'/s);
  assert.match(panels, /h\('div\.assets-top'/);
  assert.match(panels, /button\.asset-import/);
  assert.doesNotMatch(panels, /asset-count|asset-list-head|asset-list-count|h\('b', 'Media'\)/,
    'the panel title replaces the duplicate Media/count and Imported/count rows');
  assert.doesNotMatch(panels, /asset-dropzone|Choose files or drop them here|dragDepth/,
    'the large dashed Add media card is removed');
  assert.match(panels, /function mediaDetails\(asset\)/);
  assert.match(panels, /button\.asset-add/);
  assert.match(panels, /button\.asset-delete/);
  assert.match(panels, /aria-selected/);
  assert.match(panels, /event\.key === 'Delete' \|\| event\.key === 'Backspace'/);
  assert.match(panels, /PM\.MediaImport\.removeAsset\(PM\.proj, as\.id\)/);
  assert.match(panels, /PM\.cmd\('addFromAsset', as\.id\)/);
  assert.match(css, /\.asset-card\{[^}]*grid-template-columns:38px minmax\(0,1fr\) auto/s);
  assert.match(css, /\.asset-list\{[^}]*overflow:auto/s);
  assert.match(css, /\.asset-card\[aria-selected="true"\]/);
});

test('the media empty state explains project persistence without a second import card', () => {
  assert.match(panels, /Images, video, and audio stay with this project/);
  assert.doesNotMatch(css, /\.asset-dropzone|\.asset-drop-icon|\.asset-drop-copy/);
});
