/** Writes ONLY to the supplied temporary directory. Corpus source stays read-only. */
import { allProjects } from '../../../../../engine/corpus/src/projects';
import { generateCorpus } from '../../../../../engine/corpus/src/generate';
import { resolveMedia, mediaRefs } from '../../../../../engine/corpus/src/media';
import { writeFileSync } from 'node:fs';
import { isForeignEffect, studioCube, objModel, scene3d } from '../../../../../engine/corpus/src/catalog';
import path from 'node:path';
const destination = process.argv[2];
if (!destination || !path.resolve(destination).startsWith('/tmp/')) throw new Error('Supply a /tmp/ output directory');
const projects = allProjects();
generateCorpus(destination, projects);
// Include implicit and intentionally absent definitions as dependencies, even
// when the corpus does not request installation of an extension fixture.
function dependencies(p: (typeof projects)[number]): string[] {
  const deps = new Set(p.extensions ?? []);
  const nativeLayers = new Set([studioCube.id, objModel.id, scene3d.sceneDefinition, ...Object.values(scene3d.definitions)]);
  for (const comp of [p.doc, ...Object.values(p.doc.comps ?? {})] as any[]) {
    for (const layer of comp.layers ?? []) {
      if (layer.type === 'extension' && !nativeLayers.has(layer.d?.definition)) deps.add(`layer:${layer.d?.definition}`);
      for (const fx of layer.fx ?? []) if (isForeignEffect(fx.type)) deps.add(`effect:${fx.type}`);
    }
  }
  return [...deps];
}
const manifest = { projects: projects.map(p => ({
  path: path.resolve(destination, 'projects', p.id, 'document.json'),
  extensions: dependencies(p),
  media: Object.fromEntries([...mediaRefs(p.doc)].filter(r => r.startsWith('media:')).map(ref => {
    const source = resolveMedia(ref.slice('media:'.length), destination);
    return [ref, path.resolve(source.path)];
  })),
})) };
writeFileSync(path.join(destination, 'integration-manifest.json'), JSON.stringify(manifest));
console.log(`${manifest.projects.length} projects prepared; ${projects.filter(p => dependencies(p).length).length} extension-dependent`);
