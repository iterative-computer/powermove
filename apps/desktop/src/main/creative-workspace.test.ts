import { mkdtemp, mkdir, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { inspectCreativeWorkspace, parseAfterEffectsWorkspace } from './creative-workspace';

const pair = (key: string, value: string) => `<prop.pair><key>${key}</key>${value}</prop.pair>`;
const list = (...pairs: string[]) => `<prop.list>${pairs.join('')}</prop.list>`;
const frame = (id: string, visible = true) => list(pair('Frame', list(pair('TabIDs', `<array><array.type><string/></array.type><string>${id}</string><string>Other tab</string></array>`), pair('CurrTab', '<int>0</int>'), pair('Vis', visible ? '<true/>' : '<false/>'), pair('VisTabState-0', '<string>private-project-content</string>'))));
const xml = (name: string) => `<prop.map>${list(pair('UserName', `<ustring>${name}</ustring>`), pair('TopLevelFrame-0', list(pair('Splitter', list(pair('Orient', '<true/>'), pair('Place', '<float>0.25</float>'), pair('Sub1', frame('AE Project')), pair('Sub2', frame('AE Composition')), pair('Sub2Vis', '<false/>'))))))}</prop.map>`;
const temporary: string[] = [];
afterEach(async () => { await Promise.all(temporary.splice(0).map(dir => rm(dir, { recursive: true, force: true }))); });

async function fixture() {
  const home = await mkdtemp(path.join(os.tmpdir(), 'powermove-creative-workspace-')); temporary.push(home);
  const directory = path.join(home, 'Library/Preferences/Adobe/After Effects/26.2');
  await mkdir(path.join(directory, 'ModifiedWorkspaces'), { recursive: true });
  await mkdir(path.join(directory, 'OriginalUserWorkspaces'), { recursive: true });
  await writeFile(path.join(directory, 'Adobe After Effects 26.2 Prefs.txt'), '"Current Workspace3" = "My setup"\n"Recent project" = "private-project-content"');
  await writeFile(path.join(directory, 'ModifiedWorkspaces/UserWorkspace.xml'), xml('My setup'));
  const options = { home, applications: path.join(home, 'Applications'), platform: 'darwin' as const };
  return { home, directory, options };
}

describe('After Effects workspace reader', () => {
  it('reads geometry, tab order and hidden groups without copying panel state', () => {
    const result = parseAfterEffectsWorkspace(xml('Motion &amp; Design'));
    expect(result.name).toBe('Motion & Design');
    expect(result.groups).toEqual([
      { panels: ['AE Project', 'Other tab'], activePanel: 'AE Project', visible: true, bounds: { x: 0, y: 0, width: .25, height: 1 }, window: 0 },
      { panels: ['AE Composition', 'Other tab'], activePanel: 'AE Composition', visible: false, bounds: { x: .25, y: 0, width: .75, height: 1 }, window: 0 }
    ]);
    expect(JSON.stringify(result)).not.toContain('private-project-content');
  });

  it('rejects external entities, excessive nesting and malformed XML', () => {
    expect(() => parseAfterEffectsWorkspace('<!DOCTYPE x SYSTEM "file:///private-file">')).toThrow();
    expect(() => parseAfterEffectsWorkspace('<prop.map><prop.list></prop.map>')).toThrow();
    expect(() => parseAfterEffectsWorkspace('<prop.list>'.repeat(90))).toThrow();
  });

  it('uses the active modified layout, ignores backup versions, and only returns tool labels', async () => {
    const { home, directory, options } = await fixture();
    await mkdir(path.join(directory, '../99.9.backup'), { recursive: true });
    await writeFile(path.join(directory, 'OriginalUserWorkspaces/UserWorkspace.xml'), xml('My setup').replace('AE Project', 'Old Panel'));
    const scripts = path.join(directory, 'Scripts/ScriptUI Panels'); await mkdir(scripts, { recursive: true });
    await writeFile(path.join(scripts, 'Timing.jsx'), 'private-script-source');
    await writeFile(path.join(scripts, 'Disabled.jsx.disabled'), 'private-script-source');
    const cep = path.join(home, 'Library/Application Support/Adobe/CEP/extensions/timing/CSXS'); await mkdir(cep, { recursive: true });
    await writeFile(path.join(cep, 'manifest.xml'), '<ExtensionManifest ExtensionBundleId="example.timing"><Host Name="AEFT"/><Menu>Timing tools</Menu></ExtensionManifest>');
    const result = await inspectCreativeWorkspace({ appId: 'after-effects' }, options);
    expect(result).toMatchObject({ status: 'ready', version: '26.2', workspace: { name: 'My setup' }, installedTools: { scriptPanels: ['Timing.jsx'] } });
    const output = JSON.stringify(result);
    expect(output).toContain('AE Project'); expect(output).not.toContain('Old Panel');
    expect(output).not.toContain('private-project-content'); expect(output).not.toContain('private-script-source');
    expect(output).not.toContain('Disabled.jsx.disabled');
    expect(await readFile(path.join(scripts, 'Timing.jsx'), 'utf8')).toBe('private-script-source');
  });

  it('never guesses a layout and lets the agent choose a saved workspace by name', async () => {
    const { options } = await fixture();
    expect(await inspectCreativeWorkspace({ appId: 'after-effects', workspaceName: 'Missing' }, options)).toMatchObject({ status: 'needs-workspace', availableWorkspaces: ['My setup'] });
    expect(await inspectCreativeWorkspace({ appId: 'after-effects', workspaceName: 'My setup' }, options)).toMatchObject({ status: 'ready' });
    await expect(inspectCreativeWorkspace({ appId: 'unknown' }, options)).rejects.toThrow('Choose After Effects');
  });

  it('does not follow linked layout files or read an unsupported platform', async () => {
    const { directory, options } = await fixture();
    await writeFile(path.join(directory, 'outside.xml'), xml('Linked layout'));
    await symlink(path.join(directory, 'outside.xml'), path.join(directory, 'ModifiedWorkspaces/linked.xml'));
    const result = await inspectCreativeWorkspace({ appId: 'after-effects', workspaceName: 'Linked layout' }, options);
    expect(result.status).toBe('needs-workspace');
    expect(await inspectCreativeWorkspace({ appId: 'after-effects' }, { ...options, platform: 'linux' })).toMatchObject({ status: 'unavailable' });
  });
});
