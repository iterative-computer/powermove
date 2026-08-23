const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const root = path.resolve(__dirname, '..');
const spatial = fs.readFileSync(path.join(root, 'js/assistant/spatial.js'), 'utf8');
const workspace = fs.readFileSync(path.join(root, 'js/core/workspace.js'), 'utf8');
const timeline = fs.readFileSync(path.join(root, 'js/ui/timeline.js'), 'utf8');
const css = fs.readFileSync(path.join(root, 'css/app.css'), 'utf8');
const native = fs.readFileSync(path.join(root, 'native/main.swift'), 'utf8');

test('Agent is a real mobile workspace panel with native pop-out eligibility', () => {
  assert.match(spatial, /PM\.registerPanel\('agent',[\s\S]*persist: true, noscroll: true/);
  assert.match(workspace, /p\('agent', \{ size: 350 \}\)/);
  assert.match(spatial, /PM\.Layout\.restorePanel\(draft, 'agent'\)/);
  assert.match(spatial, /PM\.Layout\.addPanel\(draft, 'agent', 'right'\)/);
  assert.match(css, /#panel-agent>\.body\{[^}]*display:flex/);
});

test('model and reasoning picker are persisted and reach the local Codex client', () => {
  for (const model of ['gpt-5.6-sol', 'gpt-5.6-terra', 'gpt-5.6-luna']) {
    assert.match(spatial, new RegExp(model.replaceAll('.', '\\.')));
    assert.match(native, new RegExp(model.replaceAll('.', '\\.')));
  }
  assert.match(spatial, /agentModel/);
  assert.match(spatial, /agentReasoningEffort/);
  assert.match(spatial, /\{ model: S\.model, reasoningEffort: S\.reasoningEffort, signal: controller\.signal \}/);
  assert.match(native, /arguments\.append\(contentsOf: \["--model", model\]\)/);
  assert.match(native, /model_reasoning_effort=/);
});

test('active agent runs remain steerable and are natively cancellable', () => {
  assert.match(spatial, /function composerMode\(phase\)/);
  assert.match(spatial, /placeholder: working \? 'Add direction while the agent works/);
  assert.match(spatial, /'aria-label': 'Stop current run'/);
  assert.match(spatial, /previousRequest\?\.abort\(\)/,
    'sending steering replaces the unfinished request');
  assert.match(spatial, /This is steering for an active run/,
    'the replacement plan receives explicit steering semantics');
  assert.match(native, /add\(self, name: "pmCodexCancel"\)/);
  assert.match(native, /cancelCodex\(requestId: requestId\)/);
  assert.match(native, /process\?\.terminate\(\)/,
    'stopped or superseded jobs do not keep consuming the native Codex client');
});

test('attachments support image context and bounded text context', () => {
  assert.match(spatial, /accept: 'image\/png,image\/jpeg,image\/webp,image\/gif,text\/plain/);
  assert.match(spatial, /file\.size > 4_000_000/);
  assert.match(spatial, /reader\.readAsDataURL\(file\)/);
  assert.match(spatial, /\(await file\.text\(\)\)\.slice\(0, 100_000\)/);
  assert.match(spatial, /const userImages = S\.requestAttachments\.filter\(item => item\.dataUrl\)/);
  assert.match(spatial, /ATTACHED FILES/);
});

test('model-authored steps render as an expandable live to-do list', () => {
  assert.match(spatial, /steps: \{ type: 'array', minItems: 1, maxItems: 6/);
  assert.match(spatial, /Return 2–6 short steps/);
  assert.match(spatial, /function renderSteps\(\)/);
  assert.match(spatial, /ol\.agent-todo/);
  assert.match(spatial, /setStepProgress/);
  assert.match(spatial, /finishSteps/);
  assert.match(css, /\.agent-todo li\.active/);
  assert.match(css, /\.agent-todo li\.complete/);
  assert.match(spatial, /S\.phase === 'working' \|\| S\.phase === 'applying' \|\| !!S\.plan/,
    'the to-do list is mounted only while planning, awaiting apply, or applying');
  assert.match(spatial, /function updateSteps\(titles, active = -1\)/,
    'model-authored step updates flow through the live status model');
});

test('agent favors executable edits and can reverse Timeline surfaces', () => {
  assert.match(spatial, /Prefer a useful executable interpretation over explaining limitations/);
  assert.match(spatial, /timeline\.surfaceOrder=reversed/);
  assert.match(workspace, /timelineSurfaceOrder/);
  assert.match(timeline, /dataset\.timelineSurfaces === 'reversed'/);
  assert.match(timeline, /reversed \? '--bg-sunken' : '--bg-panel'/);
});

test('elevated prompt UI exposes scope, suggestions, and direct panel authority', () => {
  assert.match(spatial, /function openScopePicker\(event\)/);
  assert.match(spatial, /Build or rearrange anything/);
  assert.match(spatial, /Auto-apply panels/);
  assert.match(spatial, /add\|restore\|hide\|move\|reorder\|resize\|resizeDock\|rename\|collapse\|expand\|popout\|dock/);
  assert.match(spatial, /function applyPanelPlan\(plan\)/);
  assert.match(spatial, /PM\.WS\.restoreSnapshot\(S\.panelRun\.checkpoint\)/);
  assert.match(css, /\.agent-composer-head/);
  assert.match(css, /\.agent-suggestions/);
});

test('Agent welcome actions use borderless surfaces while retaining focus behavior', () => {
  const iconRule = css.match(/\.agent-welcome-icon\{[^}]*\}/)?.[0] || '';
  const suggestionRule = css.match(/\.agent-suggestions button\{[^}]*\}/)?.[0] || '';
  const suggestionHover = css.match(/\.agent-suggestions button:hover\{[^}]*\}/)?.[0] || '';
  assert.doesNotMatch(iconRule, /border:/);
  assert.doesNotMatch(suggestionRule, /border:/);
  assert.doesNotMatch(suggestionHover, /border-color:/);
  assert.match(suggestionRule, /background:color-mix\(in srgb,var\(--tx\) 4%,var\(--bg-panel-2\)\)/,
    'the borderless tiles retain a quiet resting fill in both themes');
  assert.match(suggestionHover, /background:color-mix\(in srgb,var\(--tx\) 7%,var\(--bg-panel-2\)\)/,
    'hover remains one subtle step stronger');
  assert.match(css, /button:focus-visible\{outline:2px solid color-mix\(in srgb,var\(--accent\)/,
    'keyboard users retain the shared focus indicator');
});
