// Execute the actual v1.2.0 digest with synthetic saved workspace/registrations.
// This supplies independent expected results; it does not capture live panels.
import { tag, evaluate } from './main-oracle.mjs';
import fs from 'node:fs';
const input = JSON.parse(fs.readFileSync(0, 'utf8'));
const source = tag('apps/desktop/src/renderer/src/legacy/assistant/harness.ts');
const body = source.slice(source.indexOf('function panelLayoutDigest()'), source.indexOf('async function rollBackLiveTransaction'));
const { panelLayoutDigest } = evaluate(body, ['panelLayoutDigest'], {
  PM: { WS: { current: input.workspace }, PANELS: Object.fromEntries(input.panels.map(panel => [panel.id, panel])) }
});
process.stdout.write(JSON.stringify(panelLayoutDigest()));
