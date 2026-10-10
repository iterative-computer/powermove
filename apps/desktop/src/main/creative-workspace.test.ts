import { mkdtemp, mkdir, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { inspectCreativeExtension, inspectCreativeWorkspace, parseAfterEffectsWorkspace } from './creative-workspace';

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

  it('discovers custom and third-party tools and reads their source on demand', async () => {
    const { home, directory, options } = await fixture();
    const scripts = path.join(directory, 'Scripts/ScriptUI Panels');
    await mkdir(scripts, { recursive: true });
    await writeFile(path.join(scripts, 'Timing.jsx'), 'function staggerLayers() { return 12; }');
    await writeFile(path.join(scripts, 'Protected.jsxbin'), 'compiled-source');
    const cep = path.join(home, 'Library/Application Support/Adobe/CEP/extensions/timing');
    await mkdir(path.join(cep, 'CSXS'), { recursive: true });
    await mkdir(path.join(cep, 'js'), { recursive: true });
    await writeFile(path.join(cep, 'CSXS/manifest.xml'), '<ExtensionManifest ExtensionBundleId="example.timing"><Host Name="AEFT"/><Menu>Timing tools</Menu></ExtensionManifest>');
    await writeFile(path.join(cep, 'js/panel.js'), 'export function alignLayers() { return 24; }');
    await writeFile(path.join(cep, '.env'), 'private-configuration');
    await writeFile(path.join(cep, 'settings.json'), 'private-settings');
    await writeFile(path.join(home, 'outside.js'), 'private-outside-source');
    await symlink(path.join(home, 'outside.js'), path.join(cep, 'js/linked.js'));
    await symlink(home, path.join(cep, 'linked-folder'));
    await writeFile(path.join(cep, 'js/oversized.js'), 'x'.repeat(512 * 1024 + 1));
    const result = await inspectCreativeWorkspace({ appId: 'after-effects' }, options);
    expect(result.status).toBe('ready');
    if (result.status !== 'ready') throw new Error('Expected saved workspace');
    const tools = result.installedTools!.tools;
    const custom = tools.find(tool => tool.name === 'Timing.jsx')!;
    const extension = tools.find(tool => tool.name === 'Timing tools')!;
    expect(custom).toMatchObject({ kind: 'script-panel', location: 'user' });
    expect(extension).toMatchObject({ kind: 'cep-extension', extensionId: 'example.timing' });
    expect(JSON.stringify(result)).not.toContain('function staggerLayers');
    expect(await inspectCreativeExtension({ appId: 'after-effects', toolId: custom.id }, options)).toMatchObject({ files: [{ path: 'Timing.jsx' }] });
    expect(await inspectCreativeExtension({ appId: 'after-effects', toolId: custom.id, path: 'Timing.jsx' }, options)).toMatchObject({ text: 'function staggerLayers() { return 12; }' });
    const tree = await inspectCreativeExtension({ appId: 'after-effects', toolId: extension.id }, options);
    expect(tree).toMatchObject({ files: expect.arrayContaining([{ path: 'js/panel.js', bytes: 44 }]) });
    expect(JSON.stringify(tree)).not.toMatch(/\.env|settings\.json|linked\.js|linked-folder|oversized\.js/);
    expect(await inspectCreativeExtension({ appId: 'after-effects', toolId: extension.id, path: 'js/panel.js' }, options)).toMatchObject({ text: 'export function alignLayers() { return 24; }' });
    for (const file of ['../outside.js', '/outside.js', 'js/linked.js', 'linked-folder/outside.js', 'js/oversized.js', '.env', 'settings.json', 'js\\panel.js']) {
      await expect(inspectCreativeExtension({ appId: 'after-effects', toolId: extension.id, path: file }, options)).rejects.toThrow();
    }
    const protectedTool = tools.find(tool => tool.name === 'Protected.jsxbin')!;
    expect(await inspectCreativeExtension({ appId: 'after-effects', toolId: protectedTool.id }, options)).toMatchObject({ files: [], status: 'metadata-only' });
    expect(await readFile(path.join(scripts, 'Timing.jsx'), 'utf8')).toBe('function staggerLayers() { return 12; }');
  });

  it('keeps same-named tools distinct and reports compiled plugins without reading binary data', async () => {
    const { directory, options } = await fixture();
    const userScripts = path.join(directory, 'Scripts/ScriptUI Panels');
    const app = path.join(options.applications, 'Adobe After Effects 2026', 'Adobe After Effects 2026.app', 'Contents');
    const appScripts = path.join(app, 'Scripts/ScriptUI Panels');
    await mkdir(userScripts, { recursive: true }); await mkdir(appScripts, { recursive: true });
    await mkdir(path.join(app, 'Plug-ins/Third party/Glow.plugin'), { recursive: true });
    await writeFile(path.join(userScripts, 'Timing.jsx'), 'user-source');
    await writeFile(path.join(appScripts, 'Timing.jsx'), 'installed-source');
    await writeFile(path.join(app, 'Plug-ins/Third party/Glow.plugin/binary'), 'private-binary-data');
    const result = await inspectCreativeWorkspace({ appId: 'after-effects' }, options);
    const tools = result.installedTools!.tools;
    const timing = tools.filter(tool => tool.name === 'Timing.jsx');
    expect(timing).toHaveLength(2);
    expect(new Set(timing.map(tool => tool.id)).size).toBe(2);
    for (const tool of timing) {
      expect(await inspectCreativeExtension({ appId: 'after-effects', toolId: tool.id, path: 'Timing.jsx' }, options))
        .toMatchObject({ text: tool.location === 'user' ? 'user-source' : 'installed-source' });
    }
    const plugin = tools.find(tool => tool.name === 'Glow.plugin')!;
    expect(plugin).toMatchObject({ kind: 'plugin' });
    expect(await inspectCreativeExtension({ appId: 'after-effects', toolId: plugin.id }, options)).toMatchObject({ status: 'metadata-only', files: [] });
    await expect(inspectCreativeExtension({ appId: 'after-effects', toolId: plugin.id, path: 'Glow.plugin/binary' }, options)).rejects.toThrow();
  });

  it('includes a custom script’s local helpers without escaping its source directory', async () => {
    const { home, directory, options } = await fixture();
    const scripts = path.join(directory, 'Scripts/ScriptUI Panels');
    await mkdir(path.join(scripts, 'lib'), { recursive: true });
    await writeFile(path.join(scripts, 'Timing.jsx'), '#include "lib/timing.jsxinc"\n#include "../../outside.js"\nfunction panel() {}');
    await writeFile(path.join(scripts, 'lib/timing.jsxinc'), '#include "../Timing.jsx"\nfunction stagger() {}');
    await writeFile(path.join(home, 'outside.js'), 'private-outside-source');
    const result = await inspectCreativeWorkspace({ appId: 'after-effects' }, options);
    const tool = result.installedTools!.tools.find(tool => tool.name === 'Timing.jsx')!;
    const tree = await inspectCreativeExtension({ appId: 'after-effects', toolId: tool.id }, options);
    if (!('files' in tree)) throw new Error('Expected a source file list.');
    expect(tree.files.map(file => file.path)).toEqual(['Timing.jsx', 'lib/timing.jsxinc']);
    expect(await inspectCreativeExtension({ appId: 'after-effects', toolId: tool.id, path: 'lib/timing.jsxinc' }, options)).toMatchObject({ text: '#include "../Timing.jsx"\nfunction stagger() {}' });
  });

  it('reads Windows ScriptUI and CEP installations and rejects unknown tool IDs', async () => {
    const { home } = await fixture();
    const appData = path.join(home, 'AppData/Roaming');
    const directory = path.join(appData, 'Adobe/After Effects/26.2');
    await mkdir(path.join(directory, 'ModifiedWorkspaces'), { recursive: true });
    await writeFile(path.join(directory, 'Adobe After Effects 26.2 Prefs.txt'), '"Current Workspace3" = "My setup"');
    await writeFile(path.join(directory, 'ModifiedWorkspaces/UserWorkspace.xml'), xml('My setup'));
    const applications = path.join(home, 'Program Files/Adobe');
    const scripts = path.join(applications, 'Adobe After Effects 2026/Support Files/Scripts/ScriptUI Panels');
    await mkdir(scripts, { recursive: true }); await writeFile(path.join(scripts, 'Timing.jsx'), 'windows-script');
    const cep = path.join(appData, 'Adobe/CEP/extensions/timing/CSXS');
    await mkdir(cep, { recursive: true });
    await writeFile(path.join(cep, 'manifest.xml'), '<ExtensionManifest ExtensionBundleId="example.timing"><Host Name="AEFT"/><Menu>Timing tools</Menu></ExtensionManifest>');
    const options = { home, appData, applications, platform: 'win32' as const, programFilesX86: path.join(home, 'Program Files (x86)') };
    const result = await inspectCreativeWorkspace({ appId: 'after-effects' }, options);
    const tools = result.installedTools!.tools;
    const script = tools.find(tool => tool.name === 'Timing.jsx')!;
    expect(await inspectCreativeExtension({ appId: 'after-effects', toolId: script.id, path: 'Timing.jsx' }, options)).toMatchObject({ text: 'windows-script' });
    expect(tools.find(tool => tool.extensionId === 'example.timing')).toBeDefined();
    await expect(inspectCreativeExtension({ appId: 'after-effects', toolId: 'f'.repeat(32) }, options)).rejects.toThrow('no longer available');
    await expect(inspectCreativeExtension({ appId: 'after-effects', toolId: script.id }, { ...options, platform: 'linux' })).rejects.toThrow('macOS and Windows');
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
