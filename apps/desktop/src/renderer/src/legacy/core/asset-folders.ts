/* Media folders, like the folders in After Effects' Project panel.
 *
 * Folders are project-level (they survive switching compositions) and live in
 * `project.assetFolders`, keyed by id. Media joins a folder through its own
 * `folder` field, so a folder that goes missing simply drops its contents back
 * to the top level instead of hiding them. */

export interface AssetFolder { id: string; name: string; parent: string | null }

type Project = Record<string, any>;

const isRecord = (value: unknown): value is Record<string, any> => !!value && typeof value === 'object' && !Array.isArray(value);

let counter = 0;
const folderId = () => `f${Date.now().toString(36)}${(counter++).toString(36)}${Math.random().toString(36).slice(2, 6)}`;

function store(project: Project): Record<string, AssetFolder> {
  if (!isRecord(project.assetFolders)) project.assetFolders = {};
  return project.assetFolders;
}

/** A folder id that names a real folder, or null for the top level. */
export function validFolder(project: Project, id: unknown): string | null {
  return typeof id === 'string' && isRecord(project?.assetFolders?.[id]) ? id : null;
}

export function folderList(project: Project): AssetFolder[] {
  const raw = isRecord(project?.assetFolders) ? project.assetFolders : {};
  return Object.entries(raw).filter(([, folder]) => isRecord(folder)).map(([id, folder]) => ({
    id, name: typeof folder.name === 'string' && folder.name.trim() ? folder.name : 'Untitled Folder',
    parent: validFolder(project, folder.parent) === id ? null : validFolder(project, folder.parent),
  }));
}

/** Folders directly inside `parent`, sorted by name. A parent chain broken by
 * a cycle in damaged data surfaces at the top level rather than vanishing. */
export function childFolders(project: Project, parent: string | null): AssetFolder[] {
  const folders = folderList(project);
  return folders.filter(folder => (inCycle(project, folder.id) ? null : folder.parent) === parent)
    .sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' }));
}

function inCycle(project: Project, id: string): boolean {
  const seen = new Set<string>();
  for (let at: string | null = id; at; at = validFolder(project, project.assetFolders[at]?.parent)) {
    if (seen.has(at)) return true;
    seen.add(at);
  }
  return false;
}

/** The folders from the top level down to `id`, inclusive. */
export function folderPath(project: Project, id: string | null): AssetFolder[] {
  const byId = new Map(folderList(project).map(folder => [folder.id, folder]));
  const path: AssetFolder[] = [];
  for (let at = validFolder(project, id); at && !path.some(folder => folder.id === at); at = byId.get(at)?.parent ?? null) {
    path.unshift(byId.get(at)!);
  }
  return path;
}

export function assetFolder(project: Project, asset: Record<string, any> | undefined): string | null {
  return validFolder(project, asset?.folder);
}

/** Media sitting directly in `folder`, recursively counted when `deep`. */
export function folderItemCount(project: Project, folder: string, deep = true): number {
  const inside = new Set([folder]);
  if (deep) {
    for (let grew = true; grew;) {
      grew = false;
      for (const child of folderList(project)) {
        if (child.parent && inside.has(child.parent) && !inside.has(child.id)) { inside.add(child.id); grew = true; }
      }
    }
  }
  let count = deep ? inside.size - 1 : childFolders(project, folder).length;
  for (const asset of Object.values(project?.assets ?? {})) {
    if (inside.has(assetFolder(project, asset as any) as string)) count++;
  }
  return count;
}

export function uniqueFolderName(project: Project, name: string, parent: string | null): string {
  const base = name.trim() || 'Untitled Folder';
  const taken = new Set(childFolders(project, parent).map(folder => folder.name.toLowerCase()));
  if (!taken.has(base.toLowerCase())) return base;
  for (let n = 2; ; n++) if (!taken.has(`${base} ${n}`.toLowerCase())) return `${base} ${n}`;
}

export function createFolder(project: Project, name: string, parent: string | null = null): string {
  const id = folderId();
  const at = validFolder(project, parent);
  store(project)[id] = { id, name: uniqueFolderName(project, name, at), parent: at };
  return id;
}

export function renameFolder(project: Project, id: string, name: string): boolean {
  const folder = validFolder(project, id) && project.assetFolders[id];
  const next = name.trim();
  if (!folder || !next || folder.name === next) return false;
  folder.name = next;
  return true;
}

/** True when `id` is `ancestor` or sits anywhere inside it. */
export function isWithin(project: Project, id: string | null, ancestor: string): boolean {
  return folderPath(project, id).some(folder => folder.id === ancestor);
}

/** Move media and folders into `target` (null is the top level). A folder
 * never moves into itself or its own descendants. Returns how many moved. */
export function moveItems(project: Project, items: { assets?: string[]; folders?: string[] }, target: string | null): number {
  const to = validFolder(project, target);
  let moved = 0;
  for (const id of items.assets ?? []) {
    const asset = project.assets?.[id];
    if (!isRecord(asset) || assetFolder(project, asset) === to) continue;
    if (to) asset.folder = to; else delete asset.folder;
    moved++;
  }
  for (const id of items.folders ?? []) {
    const folder = validFolder(project, id) && project.assetFolders[id];
    if (!folder || (to && isWithin(project, to, id)) || validFolder(project, folder.parent) === to) continue;
    folder.name = uniqueFolderName(project, folder.name, to);
    folder.parent = to;
    moved++;
  }
  return moved;
}

/** Delete a folder. Its media and subfolders move up to its parent, as when a
 * Finder folder is ungrouped; no media is ever removed from the project. */
export function deleteFolder(project: Project, id: string): boolean {
  if (!validFolder(project, id)) return false;
  const parent = validFolder(project, project.assetFolders[id].parent);
  for (const asset of Object.values(project.assets ?? {}) as any[]) {
    if (isRecord(asset) && asset.folder === id) { if (parent) asset.folder = parent; else delete asset.folder; }
  }
  for (const folder of Object.values(project.assetFolders) as any[]) {
    if (isRecord(folder) && folder.parent === id) folder.parent = parent;
  }
  delete project.assetFolders[id];
  return true;
}
