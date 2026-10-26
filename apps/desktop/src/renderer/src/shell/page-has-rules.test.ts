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

  /* `.a:not(:has(> .b)) > :last-child` restyled the whole page (3,800
     elements, 11 ms) whenever any node anywhere was inserted or removed:
     Chromium invalidates body's subtree for a negated structural `:has()`.
     A negated pointer state (`:not(:has(button:hover))`) changes only with
     that state. The app's stylesheets apply for the whole session. */
  it('never negate a structural :has()', () => {
    const negated = sheets.filter((file) => file.endsWith('.css')).flatMap((file) =>
      [...readFileSync(file, 'utf8').matchAll(/:not\(\s*:has\(([^)]*)\)/g)]
        .filter((m) => !/:(?:hover|active|focus(?:-visible|-within)?)\b/.test(m[1]!))
        .map((m) => `${path.basename(file)}: ${m[0]}`)
    );
    expect(negated).toEqual([]);
  });
});
