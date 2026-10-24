import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

/* A `body:has(<descendant>)` rule makes Chromium recheck it, and restyle the
   page under body, whenever a node anywhere in the page is inserted or
   removed (a timecode's text, a label): about 1,200 elements every playback
   frame. The screens these rules look for are body's children, so `> `
   keeps the recheck to body's own child list. */
describe('page :has() rules', () => {
  const desktop = path.resolve(__dirname, '../../../..');
  const sheets = [
    ...readdirSync(path.join(desktop, 'css')).filter((f) => f.endsWith('.css')).map((f) => path.join(desktop, 'css', f)),
    path.join(desktop, 'src/renderer/index.html')
  ];

  it('look for body\'s children only', () => {
    const descendant = sheets.flatMap((file) =>
      [...readFileSync(file, 'utf8').matchAll(/\b(?:body|html|:root):has\(\s*(?!>)[^)]*\)/g)].map((m) => `${path.basename(file)}: ${m[0]}`)
    );
    expect(descendant).toEqual([]);
  });
});
