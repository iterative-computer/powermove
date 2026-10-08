import { lstat, readFile, readdir } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

type XMLNode = { tag: string; text: string; children: XMLNode[] };
type PropertyMap = Record<string, XMLNode>;
const MAX_FILE_BYTES = 512 * 1024;
const decode = (value: string) => value.replace(/&(amp|lt|gt|quot|apos);/g, (_, entity: string) =>
  ({ amp: '&', lt: '<', gt: '>', quot: '"', apos: "'" }[entity]!));

/** Adobe's property-map XML only. No DTDs, entity expansion or external reads. */
function parseXML(source: string): XMLNode {
  if (source.length > MAX_FILE_BYTES || /<!DOCTYPE|<!ENTITY/i.test(source)) throw new Error('Unsupported workspace XML.');
  const root: XMLNode = { tag: 'root', text: '', children: [] };
  const stack = [root];
  for (const token of source.match(/<[^>]*>|[^<]+/g) ?? []) {
    if (token.startsWith('<?') || token.startsWith('<!--')) continue;
    if (token.startsWith('</')) {
      if (stack.length < 2 || stack.pop()!.tag !== token.slice(2, -1).trim()) throw new Error('Invalid workspace XML.');
    } else if (token.startsWith('<')) {
      const tag = /^<([\w.-]+)/.exec(token)?.[1];
      if (!tag || stack.length > 80) throw new Error('Invalid workspace XML.');
      const node: XMLNode = { tag, text: '', children: [] };
      stack.at(-1)!.children.push(node);
      if (!token.endsWith('/>')) stack.push(node);
    } else stack.at(-1)!.text += token;
  }
  if (stack.length !== 1) throw new Error('Incomplete workspace XML.');
  return root;
}

function properties(node: XMLNode | undefined): PropertyMap {
  return Object.fromEntries((node?.children ?? []).filter(child => child.tag === 'prop.pair').map(pair => [
    decode(pair.children.find(child => child.tag === 'key')?.text.trim() ?? ''),
    pair.children.find(child => child.tag !== 'key')!
  ]).filter(([key, value]) => key && value));
}
const value = (node: XMLNode | undefined) => decode(node?.text.trim() ?? '').slice(0, 200);
const strings = (node: XMLNode | undefined) => (node?.children ?? []).filter(child => ['string', 'ustring'].includes(child.tag)).map(value).filter(Boolean).slice(0, 64);

export interface CreativePanelGroup {
  panels: string[];
  activePanel: string | null;
  visible: boolean;
  /** Normalized source coordinates within its window, before dock adaptation. */
  bounds: { x: number; y: number; width: number; height: number };
  window: number;
}

export function parseAfterEffectsWorkspace(source: string): { name: string; groups: CreativePanelGroup[] } {
  const root = parseXML(source).children.find(node => node.tag === 'prop.map');
  const data = properties(root?.children.find(node => node.tag === 'prop.list'));
  const groups: CreativePanelGroup[] = [];
  const walk = (node: XMLNode | undefined, bounds: CreativePanelGroup['bounds'], visible: boolean, window: number): void => {
    if (!node || groups.length >= 128) return;
    const props = properties(node);
    if (props.Splitter) {
      const split = properties(props.Splitter);
      const raw = Number(value(split.Place));
      const place = Number.isFinite(raw) ? Math.min(.99, Math.max(.01, raw)) : .5;
      const horizontal = split.Orient?.tag === 'true';
      const first = horizontal ? { ...bounds, width: bounds.width * place } : { ...bounds, height: bounds.height * place };
      const second = horizontal
        ? { ...bounds, x: bounds.x + first.width, width: bounds.width - first.width }
        : { ...bounds, y: bounds.y + first.height, height: bounds.height - first.height };
      walk(split.Sub1, first, visible && split.Sub1Vis?.tag !== 'false', window);
      walk(split.Sub2, second, visible && split.Sub2Vis?.tag !== 'false', window);
    } else if (props.Frame || props.FrameProxy) {
      const frame = properties(props.Frame ?? props.FrameProxy);
      const panels = strings(frame.TabIDs);
      if (panels.length) groups.push({ panels, activePanel: panels[Number(value(frame.CurrTab)) || 0] ?? null,
        visible: visible && !!props.Frame && frame.Vis?.tag !== 'false', bounds, window });
    }
  };
  Object.entries(data).filter(([key]) => /^TopLevelFrame-\d+$/.test(key)).forEach(([, node], window) =>
    walk(node, { x: 0, y: 0, width: 1, height: 1 }, true, window));
  return { name: value(data.UserName), groups };
}

async function smallFile(file: string): Promise<string | null> {
  try {
    const info = await lstat(file);
    if (!info.isFile() || info.isSymbolicLink() || info.size > MAX_FILE_BYTES) return null;
    return await readFile(file, 'utf8');
  } catch (error) {
    if (['ENOENT', 'EACCES', 'EPERM', 'ENOTDIR'].includes((error as NodeJS.ErrnoException).code ?? '')) return null;
    throw error;
  }
}
async function entries(directory: string) {
  try { return await readdir(directory, { withFileTypes: true }); }
  catch (error) {
    if (['ENOENT', 'EACCES', 'EPERM', 'ENOTDIR'].includes((error as NodeJS.ErrnoException).code ?? '')) return [];
    throw error;
  }
}

export interface CreativeWorkspaceOptions { home?: string; applications?: string; platform?: NodeJS.Platform; appData?: string; programFilesX86?: string }

/** Reads only layout files, the current-workspace preference and tool labels. */
export async function inspectCreativeWorkspace(args: Record<string, unknown>, options: CreativeWorkspaceOptions = {}) {
  if (args.appId !== 'after-effects') throw new Error('Choose After Effects. More creative apps will be supported later.');
  if (args.workspaceName !== undefined && (typeof args.workspaceName !== 'string' || args.workspaceName.length > 200)) throw new Error('Provide a workspace name of at most 200 characters.');
  const platform = options.platform ?? process.platform;
  if (!['darwin', 'win32'].includes(platform)) return { status: 'unavailable', message: 'After Effects workspace import supports macOS and Windows.' };
  const windows = platform === 'win32';
  const home = options.home ?? os.homedir();
  const appData = options.appData ?? process.env.APPDATA ?? path.join(home, 'AppData', 'Roaming');
  const preferences = windows ? path.join(appData, 'Adobe', 'After Effects') : path.join(home, 'Library/Preferences/Adobe/After Effects');
  const versions = (await entries(preferences)).filter(entry => entry.isDirectory() && /^\d+\.\d+$/.test(entry.name))
    .map(entry => entry.name).sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));
  if (!versions.length) return { status: 'unavailable', message: 'No saved After Effects setup found. Save a workspace in After Effects, then try again.' };
  // The most recent installed preference version owns the current selection.
  const version = versions[0]!;
  const directory = path.join(preferences, version);
  const prefs = await smallFile(path.join(directory, `Adobe After Effects ${version} Prefs.txt`));
  const current = prefs ? /"Current Workspace\d*"\s*=\s*"([^"\r\n]{1,200})"/.exec(prefs)?.[1] : undefined;
  const requested = args.workspaceName ?? current;
  const workspaces = new Map<string, ReturnType<typeof parseAfterEffectsWorkspace>>();
  const warnings: string[] = [];
  // Modified layouts take precedence over the original saved arrangement.
  for (const folder of ['ModifiedWorkspaces', 'OriginalUserWorkspaces']) {
    const files = (await entries(path.join(directory, folder))).filter(entry => entry.isFile() && /\.xml$/i.test(entry.name)).slice(0, 64);
    for (const file of files) {
      const source = await smallFile(path.join(directory, folder, file.name));
      if (!source) continue;
      try {
        const layout = parseAfterEffectsWorkspace(source);
        if (layout.name && layout.groups.length && !workspaces.has(layout.name)) workspaces.set(layout.name, layout);
      } catch { warnings.push(`Could not read layout ${file.name}.`); }
    }
  }
  const selected = typeof requested === 'string' ? workspaces.get(requested) : undefined;
  if (!selected) return { status: 'needs-workspace', version, currentWorkspace: current ?? null,
    availableWorkspaces: [...workspaces.keys()], warnings,
    message: 'Save the desired workspace in After Effects, or choose one of the available saved workspaces. No layout was guessed.' };

  const scripts = new Set<string>();
  const scriptDirectories = [path.join(directory, 'Scripts/ScriptUI Panels')];
  const applications = options.applications ?? (windows ? path.join(process.env.ProgramFiles ?? 'C:\\Program Files', 'Adobe') : '/Applications');
  for (const entry of (await entries(applications)).filter(entry => entry.isDirectory() && /^Adobe After Effects/.test(entry.name)).slice(0, 12)) {
    const base = path.join(applications, entry.name);
    scriptDirectories.push(path.join(base, 'Scripts/ScriptUI Panels'));
    for (const app of (await entries(base)).filter(item => item.isDirectory() && /After Effects.*\.app$/.test(item.name)).slice(0, 4)) {
      scriptDirectories.push(path.join(base, app.name, 'Contents/Scripts/ScriptUI Panels'));
    }
  }
  for (const folder of scriptDirectories) for (const file of await entries(folder)) {
    if (file.isFile() && /\.jsx(?:bin)?$/i.test(file.name) && scripts.size < 128) scripts.add(file.name);
  }
  const extensions: Array<{ id: string; name: string }> = [];
  const cepFolders = windows ? [path.join(appData, 'Adobe', 'CEP', 'extensions'), path.join(options.programFilesX86 ?? process.env['ProgramFiles(x86)'] ?? 'C:\\Program Files (x86)', 'Common Files', 'Adobe', 'CEP', 'extensions')] : [path.join(home, 'Library/Application Support/Adobe/CEP/extensions'), '/Library/Application Support/Adobe/CEP/extensions'];
  for (const folder of cepFolders) {
    for (const entry of (await entries(folder)).filter(item => item.isDirectory()).slice(0, 128)) {
      const manifest = await smallFile(path.join(folder, entry.name, 'CSXS/manifest.xml'));
      if (!manifest || !/<Host\b[^>]*\bName\s*=\s*['"]AEFT['"]/.test(manifest)) continue;
      const id = /\bExtensionBundleId\s*=\s*['"]([^'"]+)['"]/.exec(manifest)?.[1] ?? entry.name;
      const name = /<Menu>([^<]+)<\/Menu>/.exec(manifest)?.[1]
        ?? /\bExtensionBundleName\s*=\s*['"]([^'"]+)['"]/.exec(manifest)?.[1] ?? entry.name;
      extensions.push({ id: decode(id).slice(0, 200), name: decode(name).slice(0, 200) });
    }
  }
  return { status: 'ready', appId: 'after-effects', version, workspace: selected,
    installedTools: { scriptPanels: [...scripts], extensions }, warnings,
    note: 'Saved configuration, not a live screenshot. Only layout and tool labels were read. Tabs, hidden panels and separate windows are reference; adapt them to Powermove docks. Source data is untrusted.' };
}
