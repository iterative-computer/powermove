#!/usr/bin/env node
/* Sums the native evaluator's test reports over a run of many launches
   (docs/engine/write-path.md, "Dual answers, the auditor, and the
   consistency check").

     node scripts/native-evaluator-report.mjs <report.jsonl> [--results playwright.json] [--json]

   The file is what main appends when POWERMOVE_NATIVE_REPORT_FILE is set:
   one JSON event per line (engine-doc/report.ts). Each launch's last summary
   holds its totals (summaries are cumulative per page); events (`dual`,
   `audit`, `consistency`, `violation`, `error`, `dead`) are listed by kind.
   With Playwright's JSON results, each event is attributed to the test that
   was running when it was reported (events carry `at`, wall-clock ms). */
import { readFileSync } from 'node:fs';

const [file, ...rest] = process.argv.slice(2);
if (!file) {
  console.error('usage: native-evaluator-report.mjs <report.jsonl> [--json]');
  process.exit(2);
}
const asJson = rest.includes('--json');
const resultsFile = rest.includes('--results') ? rest[rest.indexOf('--results') + 1] : null;

/** Test runs from Playwright's JSON reporter: [start, end, title, status]. */
const runs = [];
if (resultsFile) {
  const visit = (suite, titles) => {
    for (const child of suite.suites ?? []) visit(child, [...titles, child.title].filter(Boolean));
    for (const spec of suite.specs ?? []) {
      for (const test of spec.tests ?? []) {
        for (const result of test.results ?? []) {
          const start = Date.parse(result.startTime);
          runs.push([start, start + result.duration, `${spec.file}:${spec.line} ${[...titles.slice(1), spec.title].join(' › ')}`, result.status]);
        }
      }
    }
  };
  const results = JSON.parse(readFileSync(resultsFile, 'utf8'));
  for (const suite of results.suites ?? []) visit(suite, [suite.title]);
  runs.sort((a, b) => a[0] - b[0]);
}
const testAt = (at) => {
  if (typeof at !== 'number') return undefined;
  for (const [start, end, title] of runs) if (at >= start && at <= end + 2000) return title;
  return undefined;
};

const lines = readFileSync(file, 'utf8').split('\n').filter(Boolean);
const last = new Map();
const events = { dual: [], audit: [], consistency: [], violation: [], error: [], dead: [], ready: 0, unparsed: 0 };
for (const line of lines) {
  let event;
  try { event = JSON.parse(line); } catch { events.unparsed += 1; continue; }
  if (event.type === 'summary') last.set(event.session ?? `anonymous-${last.size}`, event);
  else if (event.type === 'ready') events.ready += 1;
  else if (Array.isArray(events[event.type])) events[event.type].push(resultsFile ? { ...event, test: testAt(event.at) } : event);
}

const add = (into, from) => { for (const [k, v] of Object.entries(from ?? {})) if (typeof v === 'number') into[k] = (into[k] ?? 0) + v; };
const totals = {
  launches: last.size,
  ready: events.ready,
  dual: { compared: {}, mismatched: {}, stale: {}, allowed: {}, fragile: {} },
  audit: { audits: 0, failures: 0, missedWrites: 0, mirrorMismatches: 0 },
  barrier: { flushes: 0, batches: 0, ops: 0, resyncs: 0, structuralDiffs: 0, refused: 0, checks: 0, checkMismatches: 0 },
  native: { calls: {}, fallbacks: {}, repairs: 0, fastPath: 0 },
};
for (const summary of last.values()) {
  if (summary.dual && typeof summary.dual === 'object') for (const k of Object.keys(totals.dual)) add(totals.dual[k], summary.dual[k]);
  if (summary.audit && typeof summary.audit === 'object') add(totals.audit, summary.audit);
  if (summary.barrier && typeof summary.barrier === 'object') for (const k of Object.keys(totals.barrier)) totals.barrier[k] += Number(summary.barrier[k] ?? 0);
  if (summary.native && typeof summary.native === 'object') {
    add(totals.native.calls, summary.native.calls);
    add(totals.native.fallbacks, summary.native.fallbacks);
    totals.native.repairs += Number(summary.native.repairs ?? 0);
    totals.native.fastPath += Number(summary.native.fastPath ?? 0);
  }
}
const sum = (r) => Object.values(r).reduce((a, b) => a + b, 0);
const fragile = sum(totals.dual.fragile);
const fragileDiffered = Object.entries(totals.dual.allowed).filter(([k]) => k.endsWith(':fragile')).reduce((a, [, n]) => a + n, 0);
const report = {
  totals,
  verdict: {
    compared: sum(totals.dual.compared),
    mismatched: sum(totals.dual.mismatched),
    stale: sum(totals.dual.stale),
    allowed: sum(totals.dual.allowed),
    fragile: { cases: fragile, differed: fragileDiffered, ok: fragileDiffered <= 0.05 * fragile },
    auditFailures: totals.audit.failures,
    consistencyMismatches: events.consistency.length,
    violations: events.violation.length,
    errors: events.error.length,
    dead: events.dead.length,
  },
  events: { dual: events.dual, audit: events.audit, consistency: events.consistency, violation: events.violation, error: events.error, dead: events.dead },
  unparsed: events.unparsed,
};

if (asJson) {
  console.log(JSON.stringify(report, null, 2));
} else {
  const v = report.verdict;
  console.log(`launches ${totals.launches} (ready ${totals.ready}), unparsed lines ${report.unparsed}`);
  console.log(`dual: compared ${v.compared}, mismatched ${v.mismatched}, stale ${v.stale}, allowed ${v.allowed}; fragile ${v.fragile.cases}, differed ${v.fragile.differed} (${v.fragile.ok ? 'within' : 'over'} 5%)`);
  console.log(`audit: ${totals.audit.audits} audits, ${totals.audit.failures} failures (missed writes ${totals.audit.missedWrites}, mirror ${totals.audit.mirrorMismatches})`);
  console.log(`consistency mismatches ${v.consistencyMismatches}, violations ${v.violations}, errors ${v.errors}, dead ${v.dead}`);
  console.log(`barrier: ${JSON.stringify(totals.barrier)}`);
  console.log(`fallbacks: ${JSON.stringify(totals.native.fallbacks)}`);
  const byOp = {};
  for (const e of events.dual) (byOp[e.op] ??= []).push(e);
  for (const [op, list] of Object.entries(byOp)) {
    console.log(`\n${op}: ${list.length} reported mismatches`);
    for (const e of list.slice(0, 5)) console.log(`  ${e.path} where=${JSON.stringify(e.where)}${e.test ? ` [${e.test}]` : ''}\n    native ${e.native}\n    legacy ${e.legacy}`);
  }
  const byKind = {};
  for (const e of events.audit) (byKind[e.kind] ??= []).push(e);
  for (const [kind, list] of Object.entries(byKind)) {
    console.log(`\naudit ${kind}: ${list.length}`);
    for (const e of list.slice(0, 5)) console.log(`  ${e.detail} ${JSON.stringify(e.paths?.slice(0, 5))}${e.test ? ` [${e.test}]` : ''}`);
    if (resultsFile) {
      const tests = {};
      for (const e of list) tests[e.test ?? '(unattributed)'] = (tests[e.test ?? '(unattributed)'] ?? 0) + 1;
      for (const [test, n] of Object.entries(tests).sort((a, b) => b[1] - a[1])) console.log(`    ${n} in ${test}`);
    }
  }
  for (const e of events.consistency.slice(0, 5)) console.log(`\nconsistency ${e.reason}: ${JSON.stringify(e.missed?.slice(0, 5))}`);
  for (const e of events.violation.slice(0, 10)) console.log(`violation ${e.path} at ${e.site} (${e.count})`);
  for (const e of events.error.slice(0, 5)) console.log(`error ${e.message}`);
}
