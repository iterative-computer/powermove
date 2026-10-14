/* Measures the expected widths of tests/blink_sizes.rs in Chromium and
   checks them against the test's values, so they can be measured again
   (for a new Chromium, or to confirm the test after a change).

   Each width is `ctx.measureText(text).width` with a `400 <size>px
   "system-ui"` canvas font, in one of two renderer states:
   - fresh: a new renderer at a device scale factor of 1 that has laid out
     no other system-ui text;
   - seeded: a new renderer at a device scale factor of 2 that first lays
     out 400 12 px and 13 px system-ui DOM text (as the app's UI does). Its
     font cache then holds 24 and 26 px system-ui entries created at the
     text optical size, which canvas text at those sizes reuses
     (`FontCacheSeed`, docs/engine/text.md).
     The scale factor comes from `--force-device-scale-factor=2`. The
     headless shell still reports `devicePixelRatio` 1 to the page, but it
     lays the DOM text out at 24 and 26 px device sizes. Playwright's
     `deviceScaleFactor` emulation does not: with it the canvas measures the
     fresh widths (checked in Chromium 151.0.7922.34).

   Uses the Chromium Playwright installs (headless shell). Starts a browser,
   so run it under the machine's Electron lock:
     ~/Library/Caches/powermove-engine/bin/pm-electron-lock \
       bun engine/crates/pm-text/scripts/blink-sizes.mjs
   Prints every case and exits non-zero when any width differs. */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const crate = join(dirname(fileURLToPath(import.meta.url)), '..');
const repo = join(crate, '../../..');
const { chromium } = createRequire(join(repo, 'apps/desktop/package.json'))('playwright');

/** The cases and expected widths, read from tests/blink_sizes.rs. */
function expectedCases() {
  const source = readFileSync(join(crate, 'tests/blink_sizes.rs'), 'utf8');
  const seedsAt = source.indexOf('c.set_font_cache_seeds(');
  if (seedsAt < 0) throw new Error('blink_sizes.rs: no set_font_cache_seeds call');
  const cases = [];
  const table = /const FRESH: &\[\(f64, f64, f64\)\] = &\[([\s\S]*?)\];/.exec(source);
  if (!table) throw new Error('blink_sizes.rs: no FRESH table');
  for (const m of table[1].matchAll(/\(([\d.]+), ([\d.]+), ([\d.]+)\)/g)) {
    cases.push({ state: 'fresh', size: Number(m[1]), text: 'H', want: Number(m[2]) });
    cases.push({ state: 'fresh', size: Number(m[1]), text: 'i', want: Number(m[3]) });
  }
  // `c.measure(&content(SIZE), "T")` and the first width literal after it.
  for (const m of source.matchAll(/c\.measure\(&content\(([\d.]+)\), "(.)"\)[^;]*?;?[\s\S]*?(\d+\.\d+)/g)) {
    cases.push({ state: m.index > seedsAt ? 'seeded' : 'fresh', size: Number(m[1]), text: m[2], want: Number(m[3]) });
  }
  return cases;
}

const SEED_HTML = `<!doctype html><body style="margin:0">
  <span style="font: 400 12px system-ui">Powermove 0123456789</span>
  <span style="font: 400 13px system-ui">Powermove 0123456789</span>
  <canvas width="1" height="1"></canvas></body>`;

async function measureIn(state, cases) {
  const browser = await chromium.launch({
    headless: true,
    args: [`--force-device-scale-factor=${state === 'seeded' ? 2 : 1}`]
  });
  try {
    const page = await browser.newPage();
    await page.setContent(state === 'seeded' ? SEED_HTML : '<!doctype html><canvas width="1" height="1"></canvas>');
    return await page.evaluate(async (list) => {
      // The DOM text is laid out (and its fonts created) before any canvas text.
      for (const span of document.querySelectorAll('span')) span.getBoundingClientRect();
      await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
      const ctx = document.querySelector('canvas').getContext('2d');
      return {
        dpr: devicePixelRatio,
        widths: list.map(({ size, text }) => {
          ctx.font = `400 ${size}px "system-ui"`;
          return ctx.measureText(text).width;
        })
      };
    }, cases).then((r) => ({ ...r, version: browser.version() }));
  } finally {
    await browser.close();
  }
}

const cases = expectedCases();
let wrong = 0;
for (const state of ['fresh', 'seeded']) {
  const these = cases.filter((c) => c.state === state);
  const { dpr, version, widths } = await measureIn(state, these);
  console.log(`${state} renderer: --force-device-scale-factor=${state === 'seeded' ? 2 : 1} (page devicePixelRatio ${dpr}), Chromium ${version}`);
  these.forEach((c, i) => {
    const same = widths[i] === c.want;
    if (!same) wrong++;
    console.log(`  ${same ? 'ok  ' : 'DIFF'} ${JSON.stringify(c.text)} at ${c.size} px: ${widths[i]}${same ? '' : ` (blink_sizes.rs ${c.want})`}`);
  });
}
console.log(`${cases.length - wrong}/${cases.length} widths equal blink_sizes.rs`);
process.exit(wrong ? 1 : 0);
