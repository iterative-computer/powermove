import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';

import { FontStore, fontType } from './fonts';

let dir: string;
afterEach(async () => { if (dir) await rm(dir, { recursive: true, force: true }); });

async function upload(root: string, name: string, bytes: Uint8Array): Promise<string> {
  const folder = path.join(root, `u-${Math.random().toString(36).slice(2)}`);
  await mkdir(folder, { recursive: true });
  const file = path.join(folder, name);
  await writeFile(file, bytes);
  return file;
}

const ttf = new Uint8Array([0, 1, 0, 0, 0, 0, 0, 0, 1, 2, 3]);
const otf = new Uint8Array([...'OTTO'].map((c) => c.charCodeAt(0)).concat([0, 0]));

describe('FontStore', () => {
  it('recognises font containers by their header', () => {
    expect(fontType(ttf)).toBe('font/ttf');
    expect(fontType(otf)).toBe('font/otf');
    expect(fontType(new Uint8Array([...'wOF2'].map((c) => c.charCodeAt(0))))).toBe('font/woff2');
    expect(fontType(new Uint8Array([1, 2, 3, 4]))).toBe('');
  });

  it('stores committed faces, lists them with system families, and serves them back', async () => {
    dir = await mkdtemp(path.join(tmpdir(), 'pm-fonts-'));
    const uploads = path.join(dir, 'uploads');
    const store = new FontStore(path.join(dir, 'Fonts'), null);
    const face = await store.commit({ family: 'Avenir Next', postscriptName: 'AvenirNext-Bold', fullName: 'Avenir Next Bold', style: 'Bold' }, await upload(uploads, 'font', ttf), uploads);
    expect(face.file).toBe('Avenir_Next/AvenirNext-Bold.ttf');
    expect(face.type).toBe('font/ttf');
    const list = await store.list();
    expect(list.families).toEqual(['Avenir Next']);
    expect(list.faces).toHaveLength(1);
    expect(await store.has(['avenir next', 'Nope'])).toEqual(['avenir next']);
    const served = await store.file('AvenirNext-Bold');
    expect(served?.type).toBe('font/ttf');
    expect([...(await readFile(served!.path))]).toEqual([...ttf]);
    // A fresh instance reads the index back.
    expect((await new FontStore(path.join(dir, 'Fonts'), null).list()).faces[0]?.postscriptName).toBe('AvenirNext-Bold');
  });

  it('refuses paths outside the upload root, non-fonts, and bad names', async () => {
    dir = await mkdtemp(path.join(tmpdir(), 'pm-fonts-'));
    const uploads = path.join(dir, 'uploads');
    const store = new FontStore(path.join(dir, 'Fonts'), null);
    const outside = await upload(path.join(dir, 'elsewhere'), 'font', ttf);
    await expect(store.commit({ family: 'A', postscriptName: 'A-Regular', fullName: '', style: '' }, outside, uploads)).rejects.toThrow(/not an upload/);
    await expect(store.commit({ family: 'A', postscriptName: 'A-Regular', fullName: '', style: '' }, await upload(uploads, 'x', new Uint8Array([9, 9, 9, 9])), uploads)).rejects.toThrow(/not a font/);
    await expect(store.commit({ family: 'A/B', postscriptName: 'A-Regular', fullName: '', style: '' }, await upload(uploads, 'x', ttf), uploads)).rejects.toThrow(/invalid face name/);
    await expect(store.commit({ family: '', postscriptName: 'A', fullName: '', style: '' }, await upload(uploads, 'x', ttf), uploads)).rejects.toThrow(/required/);
  });
});
