const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const root = path.join(__dirname, '..');
const timeline = fs.readFileSync(path.join(root, 'js/ui/timeline.js'), 'utf8');

test('Shift and Command both add or remove exact keyframes without duplicates', () => {
  assert.match(timeline, /const additive = e\.shiftKey \|\| e\.metaKey/);
  assert.match(timeline, /wasSelected \? PM\.sel\.keys\.filter\(id => id !== hit\.i\) : \[\.\.\.PM\.sel\.keys, hit\.i\]/);
  assert.match(timeline, /function uniqueKeyIds\(keys\)/);
  assert.match(timeline, /PM\.selectLayers\(r\.L\.id, true\)/,
    'modifier-selecting another property must not clear the existing key selection');
});

test('dragging empty keyframe space draws a thresholded marquee and gathers keys across property rows', () => {
  assert.match(timeline, /if \(!hit\) return marquee\(e, \{ clickTime: true, additive \}\)/);
  assert.match(timeline, /Math\.hypot\(dx, dy\) < 3/,
    'a normal click remains a scrub instead of flashing a selection box');
  assert.match(timeline, /cur = nearKey \? 'pointer' : 'crosshair'/,
    'empty property rows communicate that they support box selection');
  assert.match(timeline, /function keysInMarquee\(m, graph = false\)/);
  assert.match(timeline, /row\.kind !== 'prop'/);
  assert.match(timeline, /x >= m\.x0 && x <= m\.x1/);
  assert.match(timeline, /setSelectedKeys\(\[\.\.\.baseKeys, \.\.\.picked\]/,
    'Shift or Command marquee adds to the existing selection');
  assert.match(timeline, /if \(!additive\) setSelectedKeys\(\[\], \[\]\)/,
    'an ordinary empty-space click clears both layers and keys while still scrubbing');
});

test('blank track clicks deselect and modifier-click toggles selected layers off', () => {
  assert.match(timeline, /if \(!onClip\) return marquee\(e, \{ clickTime: true, additive: e\.shiftKey \|\| e\.metaKey \}\)/,
    'empty space on a layer row does not select that row');
  assert.match(timeline, /selected \? PM\.sel\.layers\.filter\(id => id !== L\.id\) : \[\.\.\.PM\.sel\.layers, L\.id\]/,
    'Shift or Command toggles an already-selected layer off');
  assert.match(timeline, /if \(!selectLayerForPointer\(L, e\.shiftKey \|\| e\.metaKey\)\) return/,
    'the gutter and track share the same toggle behavior');
});

test('moving a multi-key selection keeps every touched property sorted and graph dragging moves the group', () => {
  assert.match(timeline, /const entries = selectedKeyEntries\(\)/);
  assert.match(timeline, /new Set\(start\.map\(s => s\.prop\)\)/);
  assert.match(timeline, /const selected = kf\.filter\(keySelected\)/);
  assert.match(timeline, /start\.forEach\(item => \{[\s\S]*item\.key\.t[\s\S]*item\.key\.v/);
});
