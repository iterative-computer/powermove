const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const root = path.join(__dirname, '..');
const spatial = fs.readFileSync(path.join(root, 'js/assistant/spatial.js'), 'utf8');
const native = fs.readFileSync(path.join(root, 'native/main.swift'), 'utf8');
const css = fs.readFileSync(path.join(root, 'css/app.css'), 'utf8');

test('Agent offers editor, project, and explicitly confirmed computer authority', () => {
  assert.match(spatial, /AGENT_ACCESS_MODES/);
  assert.match(spatial, /id: 'editor'/);
  assert.match(spatial, /id: 'project'/);
  assert.match(spatial, /id: 'computer'/);
  assert.match(spatial, /Allow computer access for this run\?/);
  assert.match(spatial, /External actions cannot be undone/);
  assert.match(spatial, /agentAccessMode/);
  assert.match(css, /\.agent-access/);
});

test('general agent requests carry a project snapshot and use a persistent native workspace', () => {
  assert.match(spatial, /mode: 'autonomous'/);
  assert.match(spatial, /projectJSON: JSON\.stringify\(PM\.proj\)/);
  assert.match(spatial, /PM\.AgentArtifacts/);
  assert.match(native, /Agent Workspaces/);
  assert.match(native, /powermove-project\.json/);
  assert.match(native, /powermove-result-schema\.json/);
  assert.match(native, /session-project\.txt/);
  assert.match(native, /session-computer\.txt/);
  assert.match(native, /Working with project files and shell tools/);
  assert.match(native, /Researching on the web/);
  assert.match(native, /Using the installed/);
});

test('project authority gets web and workspace tools while full computer access stays explicit', () => {
  assert.match(native, /"--search"/);
  assert.match(native, /"--sandbox", "workspace-write", "--approve-for-me"/);
  assert.match(native, /"--dangerously-bypass-approvals-and-sandbox"/);
  assert.match(native, /access == "computer"/);
  assert.match(native, /Place every deliverable file under the artifacts directory/);
});

test('agent artifacts are validated, revealable, and import through normal project media', () => {
  assert.match(native, /pmAgentArtifact/);
  assert.match(native, /pmAgentReveal/);
  assert.match(native, /validatedAgentArtifactURL/);
  assert.match(native, /64 \* 1024 \* 1024/);
  assert.match(spatial, /PM\.importFiles\(\[file\]\)/);
  assert.match(spatial, /Add to timeline/);
  assert.match(spatial, /Reveal in Finder/);
  assert.match(css, /\.agent-artifact/);
});

test('autonomous project edits still pass through typed source transactions and checkpoints', () => {
  assert.match(spatial, /PM\.AgentHarness\.cleanCommand/);
  assert.match(spatial, /PM\.Edit\.apply\(commands/);
  assert.match(spatial, /PM\.hist\.squash/);
  assert.match(spatial, /Before autonomous agent/);
  assert.match(spatial, /Project changed while the autonomous agent was working/);
});
