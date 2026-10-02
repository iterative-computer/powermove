import { describe, expect, it } from 'vitest';
import {
  assetFolder, childFolders, createFolder, deleteFolder, folderItemCount, folderPath, moveItems, renameFolder,
} from './asset-folders';
import { pickedFolders } from './folder-import';

const project = (): Record<string, any> => ({ assets: { a: { id: 'a', name: 'a.png' }, b: { id: 'b', name: 'b.png' } } });

describe('media folders', () => {
  it('nests folders, files media into them and counts their contents', () => {
    const p = project();
    const shots = createFolder(p, 'Shots');
    const plates = createFolder(p, 'Plates', shots);
    expect(moveItems(p, { assets: ['a'] }, plates)).toBe(1);
    expect(assetFolder(p, p.assets.a)).toBe(plates);
    expect(childFolders(p, null).map(f => f.name)).toEqual(['Shots']);
    expect(folderPath(p, plates).map(f => f.name)).toEqual(['Shots', 'Plates']);
    expect(folderItemCount(p, shots)).toBe(2); // Plates and a.png inside it
  });

  it('gives sibling folders distinct names and never moves a folder into itself', () => {
    const p = project();
    const one = createFolder(p, 'Footage');
    const two = createFolder(p, 'Footage');
    expect(p.assetFolders[two].name).toBe('Footage 2');
    const inner = createFolder(p, 'Inner', one);
    expect(moveItems(p, { folders: [one] }, inner)).toBe(0);
    expect(moveItems(p, { folders: [one] }, one)).toBe(0);
    expect(renameFolder(p, one, '  ')).toBe(false);
    expect(renameFolder(p, one, 'Renamed')).toBe(true);
  });

  it('deleting a folder moves its contents up instead of removing media', () => {
    const p = project();
    const outer = createFolder(p, 'Outer');
    const inner = createFolder(p, 'Inner', outer);
    moveItems(p, { assets: ['a', 'b'] }, inner);
    expect(deleteFolder(p, inner)).toBe(true);
    expect(assetFolder(p, p.assets.a)).toBe(outer);
    expect(deleteFolder(p, outer)).toBe(true);
    expect(Object.keys(p.assets)).toEqual(['a', 'b']);
    expect(p.assets.a.folder).toBeUndefined();
  });

  it('treats media filed in a missing folder as top level', () => {
    const p = project();
    p.assets.a.folder = 'gone';
    expect(assetFolder(p, p.assets.a)).toBeNull();
  });
});

describe('picked folder trees', () => {
  // webkitRelativePath is read-only on File, so define it explicitly.
  const picked = (path: string) => {
    const f = new File([''], path.split('/').pop()!);
    Object.defineProperty(f, 'webkitRelativePath', { value: path });
    return f;
  };

  it('rebuilds nested folders and skips hidden files', () => {
    const trees = pickedFolders([
      picked('Shoot/b.png'), picked('Shoot/.DS_Store'), picked('Shoot/frames/f2.png'), picked('Shoot/frames/f10.png'), picked('Shoot/a.png'),
    ]);
    expect(trees.map(t => t.name)).toEqual(['Shoot']);
    expect(trees[0]!.files.map(f => f.name)).toEqual(['a.png', 'b.png']);
    expect(trees[0]!.folders[0]!.name).toBe('frames');
    expect(trees[0]!.folders[0]!.files.map(f => f.name)).toEqual(['f2.png', 'f10.png']);
  });
});
