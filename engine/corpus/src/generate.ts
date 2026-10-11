/**
 * Synthetic corpus generator.
 *
 *   bun engine/corpus/src/generate.ts --out <dir>   write documents and generated media
 *   bun engine/corpus/src/generate.ts --check       validate every document, write nothing
 *
 * Output layout: <dir>/projects/<id>/document.json, <dir>/media/<file>,
 * <dir>/index.json. Generation is deterministic: two runs produce identical bytes.
 */
import { createHash } from 'node:crypto';
import { mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { GENERATED_MEDIA, mediaRefs, MISSING_MEDIA } from './media';
import { allProjects, type CorpusProject } from './projects';
import { validateDocument, type Issue } from './validate';

export interface GeneratedProject {
  id: string; group: string; title: string; covers: string[]; samples: number[]; alpha: boolean;
  size: [number, number]; media: string[]; documentSha256: string; notes?: string;
  layerTypes: string[];
}

export interface GeneratedCorpus { projects: GeneratedProject[]; media: Record<string, { bytes: number; sha256: string }> }

export const stableJson = (value: unknown): string => JSON.stringify(value, null, 1) + '\n';
const sha256 = (bytes: string | Uint8Array) => createHash('sha256').update(bytes).digest('hex');

function layerTypesOf(doc: any): string[] {
  const types = new Set<string>();
  for (const comp of [doc, ...Object.values<any>(doc.comps ?? {})]) for (const layer of comp.layers) {
    types.add(layer.type === 'extension' ? `extension:${layer.d.definition}` : layer.type);
  }
  return [...types].sort();
}

export function describeProject(item: CorpusProject): GeneratedProject {
  const document = stableJson(item.doc);
  return {
    id: item.id, group: item.group, title: item.title, covers: item.covers, samples: item.samples, alpha: !!item.alpha,
    size: item.size ?? [item.doc.w, item.doc.h],
    media: [...mediaRefs(item.doc)].sort(), documentSha256: sha256(document),
    ...(item.notes ? { notes: item.notes } : {}), layerTypes: layerTypesOf(item.doc),
  };
}

export function validateAll(projects: CorpusProject[]): Array<{ id: string; issues: Issue[] }> {
  return projects.map(item => ({ id: item.id, issues: validateDocument(item.doc) })).filter(result => result.issues.length);
}

export function generateCorpus(outDir: string, projects = allProjects()): GeneratedCorpus {
  const failures = validateAll(projects);
  if (failures.length) {
    const lines = failures.flatMap(failure => failure.issues.slice(0, 12).map(issue => `${failure.id}: ${issue.path}: ${issue.message}`));
    throw new Error(`${failures.length} corpus documents failed validation:\n${lines.join('\n')}`);
  }
  mkdirSync(path.join(outDir, 'media'), { recursive: true });
  const media: GeneratedCorpus['media'] = {};
  for (const [name, make] of Object.entries(GENERATED_MEDIA)) {
    const bytes = make();
    writeFileSync(path.join(outDir, 'media', name), bytes);
    media[name] = { bytes: bytes.length, sha256: sha256(bytes) };
  }
  const described = projects.map(item => {
    const dir = path.join(outDir, 'projects', item.id);
    mkdirSync(dir, { recursive: true });
    writeFileSync(path.join(dir, 'document.json'), stableJson(item.doc));
    return describeProject(item);
  });
  const corpus = { projects: described, media };
  writeFileSync(path.join(outDir, 'index.json'), stableJson(corpus));
  return corpus;
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const projects = allProjects();
  if (args.includes('--check')) {
    const failures = validateAll(projects);
    for (const failure of failures) for (const issue of failure.issues) console.error(`${failure.id}: ${issue.path}: ${issue.message}`);
    const refs = new Set(projects.flatMap(item => [...mediaRefs(item.doc)]));
    console.log(`${projects.length} projects, ${failures.length} invalid, media referenced: ${[...refs].sort().join(', ')}; missing-media placeholder ${MISSING_MEDIA}`);
    process.exit(failures.length ? 1 : 0);
  }
  const at = args.indexOf('--out');
  if (at < 0 || !args[at + 1]) { console.error('usage: generate.ts --out <dir> | --check'); process.exit(2); }
  const out = path.resolve(args[at + 1]!);
  const corpus = generateCorpus(out, projects);
  console.log(`Wrote ${corpus.projects.length} projects and ${Object.keys(corpus.media).length} generated media files to ${out}`);
}
