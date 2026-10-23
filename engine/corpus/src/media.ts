/**
 * Media the corpus documents reference. Documents never embed asset ids: a
 * layer names its source as `media:<file>`, and the capture tool imports that
 * file through the app's real import path and rewrites the reference to the
 * asset id the app assigned. Fixture media comes from apps/desktop/e2e/fixtures;
 * generated media is synthesized deterministically by the generator.
 */
import { createHash } from 'node:crypto';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fixturesDir, repoRoot } from './catalog';
import { corpusGlb, corpusMaterialsGlb, corpusObj, corpusSvg } from './glb';
import { GENERATED_STILLS } from './stills';
import { GENERATED_VIDEOS } from './videos';

export const MEDIA_PREFIX = 'media:';
/** A reference no file satisfies, for the missing-model behavior. */
export const MISSING_MEDIA = 'media-missing:absent-model.glb';

export const ref = (file: string): string => MEDIA_PREFIX + file;

export interface FixtureInfo {
  file: string; kind: string; hasAlpha?: boolean; skipped?: string | null;
  expectedColorAt?: { t: number; rgb: number[] }; expectedColorAt2?: { t: number; rgb: number[] };
}

export function fixtureManifest(): FixtureInfo[] {
  return JSON.parse(readFileSync(path.join(fixturesDir, 'fixtures.json'), 'utf8'));
}

/** Every fixture file shipped in the directory, including those not listed in fixtures.json. */
export const FIXTURE_FILES = [
  'h264-aac.mp4', 'h264-aac.mov', 'hevc.mov', 'hevc.mp4', 'av1.mp4', 'prores422.mov', 'prores4444-alpha.mov',
  'hevc-alpha.mov', 'vp8-opus.webm', 'vp9-opus.webm', 'converted.mkv',
  'animated.gif', 'animated-apng.png', 'animated.webp',
  'still-red.png', 'still-alpha.png', 'still.webp', 'still.avif', 'still.heic', 'still.tiff',
  'tone.wav', 'tone.mp3', 'tone.m4a', 'tone.flac', 'tone.opus',
] as const;

export const GENERATED_MEDIA: Record<string, () => Uint8Array> = {
  'corpus-model.glb': corpusGlb,
  'corpus-materials.glb': corpusMaterialsGlb,
  'corpus-gem.obj': () => new TextEncoder().encode(corpusObj()),
  'corpus-art.svg': () => new TextEncoder().encode(corpusSvg()),
  ...GENERATED_STILLS,
  ...GENERATED_VIDEOS,
};

export interface MediaSource { name: string; origin: 'fixture' | 'generated'; path: string; bytes: number; sha256: string }

/** pm-render's synthetic still-class fixtures (PQ/HLG/cHRM PNG, CMYK JPEG, TIFF structures, AVIF classes), by file name. */
export const STILLS2_DIR = path.join(repoRoot, 'engine/crates/pm-render/tests/fixtures/stills2');
const isStills2 = (name: string) => !name.includes('/') && /\.(png|jpe?g|tiff|avif)$/i.test(name) && existsSync(path.join(STILLS2_DIR, name));

/** Resolve a `media:` name to a file: fixtures from the repository, generated files from `<corpusDir>/media` (generate.ts output). */
export function resolveMedia(name: string, corpusDir: string): MediaSource {
  const fixture = path.join(fixturesDir, name);
  if ((FIXTURE_FILES as readonly string[]).includes(name)) {
    if (!existsSync(fixture)) throw new Error(`Fixture ${name} is missing from ${fixturesDir}`);
    return describe(name, 'fixture', fixture);
  }
  if (GENERATED_MEDIA[name]) return describe(name, 'generated', path.join(corpusDir, 'media', name));
  if (isStills2(name)) return describe(name, 'fixture', path.join(STILLS2_DIR, name));
  throw new Error(`Unknown corpus media ${name}`);
}

function describe(name: string, origin: MediaSource['origin'], file: string): MediaSource {
  const bytes = readFileSync(file);
  return { name, origin, path: path.relative(repoRoot, file).startsWith('..') ? file : path.relative(repoRoot, file), bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') };
}

/** Every `media:` reference anywhere in a document. */
export function mediaRefs(value: unknown, out = new Set<string>()): Set<string> {
  if (typeof value === 'string') { if (value.startsWith(MEDIA_PREFIX)) out.add(value.slice(MEDIA_PREFIX.length)); }
  else if (Array.isArray(value)) value.forEach(item => mediaRefs(item, out));
  else if (value && typeof value === 'object') Object.values(value).forEach(item => mediaRefs(item, out));
  return out;
}
