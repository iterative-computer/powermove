// End-to-end build-tool -> actual V8 CLI test with an original public synthetic extension.
import assert from "node:assert/strict";
import { CLOCK_SOURCE } from "./clock-source.mjs";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
const [reference, runner] = process.argv.slice(2);
assert(
  reference && runner,
  "Pass the prepared reference checkout and native runner",
);
const root = await mkdtemp("/tmp/pm-extcheck/bundle-test-");
try {
  const input = path.join(root, "input"),
    out = path.join(root, "output");
  await mkdir(input);
  await mkdir(out);
  // No entry property: the reference registry's default is index.ts.
  await writeFile(
    path.join(input, "manifest.json"),
    JSON.stringify({
      id: "synthetic-build",
      name: "Synthetic",
      version: "1.0.0",
      apiVersion: 3,
    }),
  );
  await writeFile(
    path.join(input, "index.ts"),
    `export default function(api){
      api.storage.set('value',17);
      api.commands.register({id:'synthetic',label:'Synthetic',run(){return api.storage.get('value')}});
      const target=document.createElement('section');document.body.append(target);
      api.onDispose(()=>target.remove());
      api.onDispose(api.host.mount(api.ui.controls.TextField,target,{
        api,get:()=>api.project.get().name,edit:{mode:'local',label:'Synthetic',set(){}}
      }));
      api.onDispose(api.host.mount(api.ui.controls.SelectField,target,{
        api,get:()=>api.project.get().name,
        options:['Synthetic extension check','Synthetic changed'],
        edit:{mode:'set',label:'Synthetic selection',set(value){api.project.get().name=value}}
      }));
    }`,
  );
  execFileSync(
    "bun",
    [
      fileURLToPath(new URL("./bundle.ts", import.meta.url)),
      reference,
      input,
      out,
    ],
    { stdio: "pipe" },
  );
  assert.equal(
    await readFile(path.join(out, "clock.js"), "utf8"),
    CLOCK_SOURCE,
    "Bun and Node must install identical fixture source bytes",
  );
  const result = JSON.parse(
    execFileSync(runner, [], {
      encoding: "utf8",
      input: JSON.stringify({
        clock_fixture: path.join(out, "clock.js"),
        compat_bundle: path.join(out, "compat.js"),
        module: path.join(out, "extension.js"),
        manifest: JSON.parse(
          await readFile(path.join(out, "manifest.json"), "utf8"),
        ),
        initial: JSON.parse(
          await readFile(path.join(out, "initial.json"), "utf8"),
        ),
        expressions: [
          "__extcheck.realm.kernel.commands.get('synthetic').run()",
          "typeof PowermoveCompat.runtimeModules.svelte.mount",
          "Array.isArray(PM.proj.markers)",
          "__extcheck.dom.document.querySelector('input').value",
          "(()=>{__extcheck.realm.history.do('Synthetic control update',()=>PM.proj.name='Synthetic changed');PowermoveCompat.runtimeModules.svelte.flushSync();return __extcheck.dom.document.querySelector('input').value})()",
          "(()=>{const select=document.querySelector('select');select.value='0';select.dispatchEvent(new Event('change',{bubbles:true}));PowermoveCompat.runtimeModules.svelte.flushSync();return [PM.proj.name,document.querySelector('input').value,select.selectedIndex]})()",
          "(()=>{__extcheck.realm.history.undo();PowermoveCompat.runtimeModules.svelte.flushSync();return [PM.proj.name,document.querySelector('input').value]})()",
          "__extcheck.dispose()",
        ],
      }),
    }),
  );
  assert.equal(result.logic, "runtime-v8");
  assert.equal(result.activation.ok, true, JSON.stringify(result.activation));
  assert.deepEqual(result.results, [
    17,
    "function",
    true,
    "Synthetic extension check",
    "Synthetic changed",
    ["Synthetic extension check", "Synthetic extension check", 0],
    ["Synthetic changed", "Synthetic changed"],
    true,
  ]);
  assert.deepEqual(result.diagnostics.missing, []);
  assert.equal(result.gates.webkit_panels, false);
  console.log(
    "PASS: default entry, compiled module global, shared compat+DOM bundle and actual V8 activation",
  );
} finally {
  await rm(root, { recursive: true, force: true });
}
