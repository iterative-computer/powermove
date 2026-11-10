// Reverse check: the shipped app at current main opens files V2 saved.
// Usage: bun reopen.mjs <original.pmv> <untouched.pmv> <edited.pmv> <history.pmv> <layerName> <editedName>
// history.pmv: V2 after main's saved history replayed (redo, undo x3, redo all, undo).
// Reads each file with main's own reader (PMV4 index and checksums), then
// compares documents with JavaScript deep equality, compares main's open-time
// hydrate of both, and imports the saved Undo history into main's editor.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {load, hydrator} from './main-ref.mjs';

const [originalPath, untouchedPath, editedPath, historyPath, layerName, editedName] = process.argv.slice(2);
const {unpackProjectFileBlob} = await load('renderer/src/legacy/core/project-file.ts');
const open = async file => {
  const document = await unpackProjectFileBlob(new Blob([readFileSync(file)]));
  const media = await Promise.all((document.containerMedia ?? []).map(async m => [m.id, m.type, Buffer.from(await m.data.arrayBuffer()).toString('base64')]));
  delete document.containerMedia;
  return {document, media};
};
const original = await open(originalPath), untouched = await open(untouchedPath), edited = await open(editedPath), replayed = await open(historyPath);
const {PM, hydrate} = await hydrator();
/** Paths at which two JSON values differ (for readable failures). */
const paths = (a, b, at = '', out = []) => {
  if (a && b && typeof a === 'object' && typeof b === 'object' && Array.isArray(a) === Array.isArray(b)) {
    for (const k of new Set([...Object.keys(a), ...Object.keys(b)])) paths(a[k], b[k], `${at}.${k}`, out);
  } else if (JSON.stringify(a) !== JSON.stringify(b)) out.push(`${at}: ${JSON.stringify(a)?.slice(0, 80)} != ${JSON.stringify(b)?.slice(0, 80)}`);
  return out;
};
const same = (a, b, message) => { const d = paths(a, b); assert.ok(!d.length, `${message}:\n${d.slice(0, 20).join('\n')}`); };

assert.deepStrictEqual(untouched.document, original.document, 'untouched V2 save differs in main');
assert.deepStrictEqual(untouched.media, original.media, 'embedded media differs in main');
assert.deepStrictEqual(edited.media, original.media, 'edited save changed embedded media');
assert.deepStrictEqual(hydrate(untouched.document.proj), hydrate(original.document.proj), 'main opens the V2 save differently');

// The edited file: main sees exactly the edit, and its Undo restores the original.
const layer = project => project.layers.find(l => l.name === layerName || l.name === editedName);
assert.equal(layer(edited.document.proj).name, editedName);
const project = hydrate(edited.document.proj);
PM.proj = project;
assert.ok(PM.hist.import(edited.document.history), 'main refused the V2 history');
assert.ok(PM.hist.canUndo?.() ?? true);
PM.hist.undo();
assert.equal(layer(PM.proj).name, layerName, "main's Undo of the V2 edit");
const reverted = structuredClone(PM.proj);
assert.deepStrictEqual({...reverted, revision: 0, edits: []}, {...hydrate(original.document.proj), revision: 0, edits: []},
  "main's Undo of the V2 edit leaves other differences");

// Main's own Undo/Redo of its saved history reaches the project V2 reached
// with the same steps, as main opens both. (Main's live keyframes carry
// normalised default ease fields its saved patches lack; its open-time
// hydrate restores them, so the comparison is through hydrate.)
PM.proj = hydrate(original.document.proj);
assert.ok(PM.hist.import(original.document.history));
PM.hist.redo(); PM.hist.undo(); PM.hist.undo(); PM.hist.undo();
for (let i = 0; i < 3; i++) PM.hist.redo();
PM.hist.undo();
same({...hydrate(replayed.document.proj), revision: 0, edits: []}, {...hydrate(PM.proj), revision: 0, edits: []},
  'V2 replays main history differently from main');
process.stderr.write('main reads the V2 saves\n');
