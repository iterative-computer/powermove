/*
 * Fonts on the host. Text renders wherever a tab is, with whatever fonts that
 * browser can see, so a project made on one machine must carry its fonts to
 * the next. A tab sends the faces of a family the host does not have (Local
 * Font Access gives the bytes); the host keeps them and serves them to any
 * tab that lacks them. "Has" means installed on the host system (fc-list)
 * or already in the store.
 */
import { execFile } from 'node:child_process';
import { mkdir, readFile, rename, copyFile, rm, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';

const run = promisify(execFile);

export interface StoredFace {
  family: string;
  postscriptName: string;
  fullName: string;
  style: string;
  file: string;
  type: string;
}

const NAME = /^[^\x00-\x1f\x7f/\\]{1,120}$/;
const safe = (value: string): string => value.replace(/[^A-Za-z0-9._-]+/g, '_').slice(0, 120) || '_';
const norm = (value: string): string => value.trim().toLocaleLowerCase();

const TYPES: Record<string, string> = { '.ttf': 'font/ttf', '.otf': 'font/otf', '.woff': 'font/woff', '.woff2': 'font/woff2', '.ttc': 'font/collection' };

export function fontType(bytes: Uint8Array): string {
  const tag = String.fromCharCode(...bytes.subarray(0, 4));
  if (tag === 'OTTO') return 'font/otf';
  if (tag === 'wOFF') return 'font/woff';
  if (tag === 'wOF2') return 'font/woff2';
  if (tag === 'ttcf') return 'font/collection';
  if (tag === 'true' || bytes[0] === 0 && bytes[1] === 1 && bytes[2] === 0 && bytes[3] === 0) return 'font/ttf';
  return '';
}

export class FontStore {
  private faces: StoredFace[] = [];
  private systemFamilies: Set<string> | null = null;
  private loaded: Promise<void> | null = null;

  constructor(private readonly dir: string, private readonly fcList: string | null = 'fc-list') {}

  private get indexPath(): string { return path.join(this.dir, 'index.json'); }

  private load(): Promise<void> {
    if (!this.loaded) {
      this.loaded = (async () => {
        try {
          const saved: unknown = JSON.parse(await readFile(this.indexPath, 'utf8'));
          if (Array.isArray(saved)) this.faces = saved.filter((face): face is StoredFace => !!face && typeof face === 'object' && typeof (face as StoredFace).family === 'string' && typeof (face as StoredFace).file === 'string');
        } catch { this.faces = []; }
      })();
    }
    return this.loaded;
  }

  private async save(): Promise<void> {
    await mkdir(this.dir, { recursive: true });
    await writeFile(`${this.indexPath}.tmp`, JSON.stringify(this.faces, null, 2));
    await rename(`${this.indexPath}.tmp`, this.indexPath);
  }

  /** Families installed on this machine, via fontconfig when present. */
  async system(): Promise<Set<string>> {
    if (this.systemFamilies) return this.systemFamilies;
    const families = new Set<string>();
    if (this.fcList) {
      try {
        const { stdout } = await run(this.fcList, ['--format', '%{family}\n'], { maxBuffer: 16 * 1024 * 1024 });
        for (const line of stdout.split('\n')) for (const family of line.split(',')) if (family.trim()) families.add(family.trim());
      } catch { /* no fontconfig: only the store counts */ }
    }
    this.systemFamilies = families;
    return families;
  }

  async list(): Promise<{ families: string[]; faces: StoredFace[] }> {
    await this.load();
    const families = new Set([...(await this.system()), ...this.faces.map((face) => face.family)]);
    return { families: [...families].sort((a, b) => a.localeCompare(b)), faces: [...this.faces] };
  }

  async has(families: string[]): Promise<string[]> {
    const { families: known } = await this.list();
    const index = new Set(known.map(norm));
    return families.filter((family) => index.has(norm(family)));
  }

  /** Moves an uploaded file into the store under its face's name. */
  async commit(face: Omit<StoredFace, 'file' | 'type'>, uploadedPath: string, uploadsRoot: string): Promise<StoredFace> {
    for (const value of [face.family, face.postscriptName, face.fullName, face.style]) {
      if (typeof value !== 'string' || !NAME.test(value) && value !== '') throw new Error('font: invalid face name');
    }
    if (!face.family || !face.postscriptName) throw new Error('font: family and postscriptName are required');
    const root = path.resolve(uploadsRoot);
    const source = path.resolve(uploadedPath);
    const relative = path.relative(root, source);
    if (!relative || relative.startsWith('..') || path.isAbsolute(relative)) throw new Error('font: path is not an upload');
    const head = Buffer.alloc(4);
    const handle = await (await import('node:fs/promises')).open(source, 'r');
    try { await handle.read(head, 0, 4, 0); } finally { await handle.close(); }
    const type = fontType(head);
    if (!type) throw new Error('font: not a font file');
    const extension = Object.entries(TYPES).find(([, value]) => value === type)?.[0] ?? '.ttf';
    await this.load();
    const familyDir = path.join(this.dir, safe(face.family));
    await mkdir(familyDir, { recursive: true });
    const file = path.join(safe(face.family), `${safe(face.postscriptName)}${extension}`);
    const target = path.join(this.dir, file);
    await rename(source, target).catch(async () => { await copyFile(source, target); });
    await rm(path.dirname(source), { recursive: true, force: true });
    const stored: StoredFace = { ...face, file, type };
    this.faces = [...this.faces.filter((item) => item.postscriptName !== face.postscriptName), stored];
    await this.save();
    return stored;
  }

  async file(postscriptName: string): Promise<{ path: string; type: string; size: number } | null> {
    await this.load();
    const face = this.faces.find((item) => item.postscriptName === postscriptName);
    if (!face) return null;
    const full = path.join(this.dir, face.file);
    try { const info = await stat(full); return { path: full, type: face.type, size: info.size }; } catch { return null; }
  }
}
