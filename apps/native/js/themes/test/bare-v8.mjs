// Executed in an isolated VM context with only V8 intrinsics. Node is the
// harness here, not an injected runtime capability.
import { SourceTextModule, createContext } from "node:vm";
import { readFileSync } from "node:fs";
import assert from "node:assert/strict";

const context = createContext({});
const cache = new Map();
function load(url) {
  if (!cache.has(url.href))
    cache.set(
      url.href,
      new SourceTextModule(readFileSync(url, "utf8"), {
        context,
        identifier: url.href,
      }),
    );
  return cache.get(url.href);
}
const runtime = load(new URL("../index.mjs", import.meta.url));
await runtime.link((specifier, referencing) => {
  assert.ok(specifier.startsWith("./"));
  return load(new URL(specifier, referencing.identifier));
});
await runtime.evaluate();
// All inputs and all allocations inside the test originate in the same realm.
const driver = new SourceTextModule(
  `
  import { translateTheme, evaluateTheme, ThemeController, ThemePublication } from './index.mjs';
  if (typeof process !== 'undefined' || typeof document !== 'undefined' || typeof structuredClone !== 'undefined') throw Error('Non-V8 global injected');
  const registry = new ThemeController();
  const owner = registry.api('fixture-owner');
  owner.register({ id: 'fixture', name: 'Fixture', scheme: 'auto', tokens: { '--accent': '#123456' }, css: '#dock{width:280px}' });
  owner.activate('fixture');
  const tree = { id: 'root', tag: 'html', children: [{ id: 'dock', tag: 'section' }] };
  const snapshot = evaluateTheme(registry.plan(), tree);
  if (snapshot.nativePatch['colors.accent'] !== 0x123456 || snapshot.surfaces.dock.base.style.width !== '280px') throw Error('V8 translation failed');
  let current;
  const publication = new ThemePublication({ replace(value) { current = value; } });
  publication.commit(publication.request(), snapshot);
  if (current.snapshot.id !== 'fixture') throw Error('V8 publication failed');
  owner.dispose();
  if (registry.selection.activeId !== 'default') throw Error('V8 cleanup failed');
`,
  { context, identifier: new URL("../bare-driver.mjs", import.meta.url).href },
);
await driver.link(() => runtime);
await driver.evaluate();
