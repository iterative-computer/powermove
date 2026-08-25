// Legacy tripwire. When js/ui/layout.js is retired this file is replaced by a rendered-DOM test over tests/fixtures/dom-contract.json.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const root = path.resolve(__dirname, '..');
const layout = fs.readFileSync(path.join(root, 'js/ui/layout.js'), 'utf8');
const spatial = fs.readFileSync(path.join(root, 'js/assistant/spatial.js'), 'utf8');
const workspace = fs.readFileSync(path.join(root, 'js/core/workspace.js'), 'utf8');
const contract = JSON.parse(fs.readFileSync(path.join(root, 'tests/fixtures/dom-contract.json'), 'utf8'));

const regexEscape = value => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const datasetKey = attribute => attribute.slice(5).replace(/-([a-z])/g, (_match, letter) => letter.toUpperCase());

function sourceBetween(source, start, end) {
  const from = source.indexOf(start);
  const to = source.indexOf(end, from + start.length);
  assert.notEqual(from, -1, `source contains ${start}`);
  assert.notEqual(to, -1, `source contains ${end} after ${start}`);
  return source.slice(from, to);
}

// Regex-on-source is intentional: these selectors and data attributes are a markup contract.
test('buildPanel emits the stable panel selectors and identity attributes', () => {
  const buildPanel = sourceBetween(layout, 'function buildPanel(', 'function applyPanelSize(');
  const panel = contract.panel;

  assert.match(buildPanel, new RegExp(`const cls = ['"]\\.${regexEscape(panel.class)}['"]`));
  assert.match(buildPanel, new RegExp(`h\\(['"]div['"] \\+ cls,\\s*\\{\\s*id:\\s*['"]${regexEscape(panel.idPrefix)}['"] \\+ spec\\.id\\s*\\}\\)`));
  assert.match(buildPanel, new RegExp(`el\\.dataset\\.${datasetKey(panel.dataAttr)}\\s*=\\s*spec\\.id`));
  assert.match(buildPanel, new RegExp(`h\\(['"]span${regexEscape(panel.title)}['"],\\s*spec\\.title \\|\\| def\\.title\\)`));
  assert.match(buildPanel, new RegExp(`h\\(['"]button${regexEscape(panel.moveHandle)}['"],`));
});

test('inspectRegion discovers panels through the stable panel selector and dataset key', () => {
  const inspectRegion = sourceBetween(spatial, 'function inspectRegion(', 'function describeElement(');
  const panel = contract.panel;
  const key = datasetKey(panel.dataAttr);

  assert.match(inspectRegion, new RegExp(`el\\.closest\\(['"]\\.${regexEscape(panel.class)}['"]\\)`));
  assert.match(inspectRegion, new RegExp(`panel\\?\\.dataset\\.${key}`));
  assert.match(inspectRegion, new RegExp(`panels\\.set\\(panel\\.dataset\\.${key},`));
});

test('applyFeatures publishes preview and Timeline surface modes as root data attributes', () => {
  const applyFeatures = sourceBetween(workspace, 'function applyFeatures(', 'WS.save =');
  const [rounded, square] = contract.documentDataset.previewCorners;
  const [reversed, normal] = contract.documentDataset.timelineSurfaces;

  assert.match(applyFeatures, new RegExp(
    `document\\.documentElement\\.dataset\\.previewCorners\\s*=\\s*` +
    `w\\.chrome\\?\\.previewCornerRadius\\s*===\\s*['"]${regexEscape(rounded)}['"]\\s*` +
    `\\?\\s*['"]${regexEscape(rounded)}['"]\\s*:\\s*['"]${regexEscape(square)}['"]`,
  ));
  assert.match(applyFeatures, new RegExp(
    `document\\.documentElement\\.dataset\\.timelineSurfaces\\s*=\\s*` +
    `w\\.chrome\\?\\.timelineSurfaceOrder\\s*===\\s*['"]${regexEscape(reversed)}['"]\\s*` +
    `\\?\\s*['"]${regexEscape(reversed)}['"]\\s*:\\s*['"]${regexEscape(normal)}['"]`,
  ));
});
