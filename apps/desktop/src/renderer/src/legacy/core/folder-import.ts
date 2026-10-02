import { bridge as hostBridge } from '../../kernel/bridge';
import { cloudSourcePaths } from './cloud-media';

/** One folder from disk: its own files and its subfolders, in name order. */
export interface ImportFolderNode { name: string; files: File[]; folders: ImportFolderNode[] }

const hiddenName = (name: string) => name.startsWith('.') || /^(thumbs\.db|desktop\.ini)$/i.test(name);
const byName = (a: { name: string }, b: { name: string }) => a.name.localeCompare(b.name, undefined, { numeric: true });
const hostPath = (file: File): string => hostBridge()?.media?.sourcePath?.(file) || '';

/** Split a drop into loose files and folders. Directory entries die with the
 * drop event, so this must run synchronously inside the handler. Returns null
 * when nothing dropped was a folder, leaving the ordinary file import as is. */
export function droppedFolders(dataTransfer: DataTransfer | null | undefined) {
  const items = Array.from(dataTransfer?.items ?? []).filter(item => item.kind === 'file');
  const files: File[] = [];
  const folders: Array<{ entry: FileSystemDirectoryEntry; path: string }> = [];
  for (const item of items) {
    const entry = item.webkitGetAsEntry?.();
    const file = item.getAsFile();
    if (entry?.isDirectory) folders.push({ entry: entry as FileSystemDirectoryEntry, path: file ? hostPath(file) : '' });
    else if (file) files.push(file);
  }
  return folders.length ? { files, folders } : null;
}

function readBatch(reader: FileSystemDirectoryReader): Promise<FileSystemEntry[]> {
  return new Promise((resolve, reject) => reader.readEntries(resolve, reject));
}

function entryFile(entry: FileSystemFileEntry): Promise<File> {
  return new Promise((resolve, reject) => entry.file(resolve, reject));
}

/** Read a dropped folder. `path` is the folder's location on disk; a file the
 * host cannot map back to disk is given its path explicitly, because media
 * conversion (image sequences, proxies) works from the original file. */
export async function readDroppedFolder(entry: FileSystemDirectoryEntry, path: string): Promise<ImportFolderNode> {
  const root = path && path.endsWith(entry.name) ? path.slice(0, path.length - entry.name.length) : '';
  const read = async (directory: FileSystemDirectoryEntry): Promise<ImportFolderNode> => {
    const reader = directory.createReader();
    const entries: FileSystemEntry[] = [];
    // readEntries returns at most ~100 entries per call.
    for (let batch = await readBatch(reader); batch.length; batch = await readBatch(reader)) entries.push(...batch);
    const node: ImportFolderNode = { name: directory.name, files: [], folders: [] };
    for (const child of entries.sort(byName)) {
      if (hiddenName(child.name)) continue;
      if (child.isDirectory) node.folders.push(await read(child as FileSystemDirectoryEntry));
      else {
        const file = await entryFile(child as FileSystemFileEntry);
        if (!hostPath(file) && root) cloudSourcePaths.set(file, root + child.fullPath.replace(/^\/+/, ''));
        node.files.push(file);
      }
    }
    return node;
  };
  return read(entry);
}

/** Rebuild folder trees from an `<input webkitdirectory>` selection. */
export function pickedFolders(files: File[]): ImportFolderNode[] {
  const roots: ImportFolderNode[] = [];
  for (const file of files) {
    const parts = (file.webkitRelativePath || file.name).split('/').filter(Boolean);
    if (parts.some(hiddenName)) continue;
    let level = roots, node: ImportFolderNode | undefined;
    for (const name of parts.slice(0, -1)) {
      node = level.find(folder => folder.name === name);
      if (!node) level.push(node = { name, files: [], folders: [] });
      level = node.folders;
    }
    if (node) node.files.push(file);
  }
  const sort = (nodes: ImportFolderNode[]) => {
    nodes.sort(byName);
    for (const node of nodes) { node.files.sort(byName); sort(node.folders); }
  };
  sort(roots);
  return roots;
}
