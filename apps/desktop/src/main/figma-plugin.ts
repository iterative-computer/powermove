import { createHash } from 'node:crypto';
import { mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
const FILES = ['manifest.json', 'dist/ui.html', 'dist/code.js'] as const;
export function figmaPluginSource(appPath: string, resourcesPath: string, packaged: boolean): string {
  return packaged ? path.join(resourcesPath, 'figma-plugin') : path.resolve(appPath, '../../packages/figma-plugin');
}
/** Stage the shipped bytes before replacing our copy; the imported path never follows
 * a versioned .app path. Only generated files in our own plugin folder are replaced. */
export async function installFigmaPlugin(source: string, userData: string): Promise<string> {
  const target = path.join(userData, 'Figma Plugin'),
    marker = '.bundle-version';
  const files = await Promise.all(
    FILES.map(async (name) => ({ name, bytes: await readFile(path.join(source, name)) })),
  );
  const hash = createHash('sha256');
  for (const f of files) hash.update(f.name).update(f.bytes);
  const version = hash.digest('hex');
  try {
    if ((await readFile(path.join(target, marker), 'utf8')) === version) {
      const installed = await Promise.all(FILES.map((name) => readFile(path.join(target, name))));
      if (installed.every((bytes, i) => bytes.equals(files[i]!.bytes))) return path.join(target, 'manifest.json');
    }
  } catch {
    /* First installation, missing files or a newer bundled plugin. */
  }
  const staging = target + '.updating';
  await rm(staging, { recursive: true, force: true });
  for (const f of files) {
    await mkdir(path.dirname(path.join(staging, f.name)), { recursive: true });
    await writeFile(path.join(staging, f.name), f.bytes);
  }
  await writeFile(path.join(staging, marker), version);
  await rm(target, { recursive: true, force: true });
  await rename(staging, target);
  return path.join(target, 'manifest.json');
}
