#!/usr/bin/env node
/*
 * Import-boundary lint for the extension architecture.
 *
 * Rule: files under src/extensions/<id>/ may import only
 *   - 'powermove' (kernel API types)
 *   - 'svelte', 'svelte/*'
 *   - relative paths that stay inside the same extension folder
 * Anything else (another extension, src/renderer internals, node builtins)
 * is a violation: it would make the extension un-forkable and let built-ins
 * couple to each other outside the public API.
 *
 * Exit 1 on any violation. Run: node scripts/check-boundaries.mjs
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import path from 'node:path';

const root = path.resolve(new URL('..', import.meta.url).pathname);
const extRoot = path.join(root, 'src', 'extensions');

const ALLOWED_BARE = [/^powermove$/, /^svelte$/, /^svelte\/.+/];
const IMPORT_RE = /(?:^|\n)\s*(?:import|export)\s[^'"]*?from\s*['"]([^'"]+)['"]|import\s*\(\s*['"]([^'"]+)['"]\s*\)|(?:^|\n)\s*import\s*['"]([^'"]+)['"]/g;

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = path.join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) walk(p, out);
    else if (/\.(ts|js|mjs|svelte)$/.test(name) && !/\.test\.ts$/.test(name)) out.push(p);
  }
  return out;
}

let violations = 0;
let extDirs = [];
try {
  extDirs = readdirSync(extRoot).filter((n) => statSync(path.join(extRoot, n)).isDirectory());
} catch {
  console.log('check-boundaries: no src/extensions directory; nothing to check');
  process.exit(0);
}

for (const id of extDirs) {
  const dir = path.join(extRoot, id);
  for (const file of walk(dir)) {
    const text = readFileSync(file, 'utf8');
    for (const m of text.matchAll(IMPORT_RE)) {
      const spec = m[1] ?? m[2] ?? m[3];
      if (!spec) continue;
      const rel = path.relative(root, file);
      if (spec.startsWith('.')) {
        const target = path.resolve(path.dirname(file), spec);
        const inside = !path.relative(dir, target).startsWith('..');
        if (!inside) {
          console.error(`${rel}: relative import escapes extension "${id}": ${spec}`);
          violations++;
        }
        continue;
      }
      if (!ALLOWED_BARE.some((re) => re.test(spec))) {
        console.error(`${rel}: disallowed import "${spec}" (only powermove, svelte, and in-folder relative imports)`);
        violations++;
      }
    }
  }
}

if (violations) {
  console.error(`check-boundaries: ${violations} violation(s)`);
  process.exit(1);
}
console.log(`check-boundaries: ok (${extDirs.length} extension(s))`);
