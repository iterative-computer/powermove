import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { makePM } from './make-pm';

function libraryModel() {
  const historyLabels: string[] = [];
  const PM = makePM('core/easing', 'core/model', 'core/library', 'gl/shaders');
  PM.time = 0;
  PM.Export = { snapshot: () => 'data:image/jpeg;base64,thumb' };
  PM.hist = {
    do(label: string, mutate: () => unknown) {
      historyLabels.push(label);
      return mutate();
    },
  };
  const shaderHooks = { syncShaderUniforms(layer: any) {
    const definitions = PM.parseUniforms(layer.d.code);
    layer._udefs = definitions;
    const uniforms = layer.d.uniforms;
    for (const definition of definitions) {
      if (!uniforms[definition.name]) uniforms[definition.name] = PM.P(definition.def);
    }
    for (const name in uniforms) {
      if (!definitions.some((definition: any) => definition.name === name)) delete uniforms[name];
    }
  } };
  PM.Kernel.services.register('shaderHooks', shaderHooks);
  PM.__historyLabels = historyLabels;
  return { PM, shaderHooks };
}

function baseProject(PM: PMRegistry): any {
  const project = PM.mkProject({ name: 'T', w: 1920, h: 1080, fps: 30, dur: 10 });
  PM.proj = project;
  return project;
}

describe('legacy library behavior survivors', () => {
  it('promotes a saved version without mutating older versions', () => {
    const { PM } = libraryModel();
    const project = baseProject(PM);
    const first = PM.mkLayer('shape', { name: 'A' }, project);
    const second = PM.mkLayer('text', { name: 'B' }, project);
    project.layers.push(first, second);
    const entry = PM.Library.saveSection('Hero', [first.id]);

    PM.Library.saveVersion(entry.id, [second.id]);

    expect(entry.versions).toHaveLength(2);
    expect(entry.versions[0].layers.map((layer: any) => layer.name)).toEqual(['A']);
    expect(entry.layers.map((layer: any) => layer.name)).toEqual(['B']);
  });

  it('inserts sections with fresh identities and shifts them to the playhead', () => {
    const { PM } = libraryModel();
    const project = baseProject(PM);
    const inner = PM.mkLayer('shape', { name: 'Inner' }, project);
    const outer = PM.mkLayer('null', { name: 'Outer' }, project);
    inner.from = 2;
    outer.from = 3;
    inner.parent = outer.id;
    project.layers.push(inner, outer);
    const entry = PM.Library.saveSection('Pair', [inner.id, outer.id]);
    project.layers.length = 0;

    PM.time = 5;
    const clones = PM.Library.insertSection(entry.id);

    expect(clones).toHaveLength(2);
    expect(clones[0].id).not.toBe(inner.id);
    expect(clones[0].from).toBe(5);
    expect(clones[1].from).toBe(6);
    expect(clones[0].parent).toBe(clones[1].id);
    expect(project.layers.map((layer: any) => layer.id).sort()).toEqual(
      clones.map((layer: any) => layer.id).sort(),
    );
  });

  it('round-trips shader code and uniform values through a saved look', () => {
    const { PM, shaderHooks } = libraryModel();
    const project = baseProject(PM);
    const shader = PM.mkLayer('shader', { name: 'Ember' }, project);
    shader.d.code = 'uniform vec3 uTint; void main(){ fragColor = vec4(uTint, 1.); }';
    shaderHooks.syncShaderUniforms(shader);
    shader.d.uniforms.uTint.v = '#123456';
    project.layers.push(shader);

    const entry = PM.Library.saveLook('Ember saved', shader);
    expect(entry.code).toContain('vec4(uTint, 1.)');
    expect(entry.uniforms.uTint.v).toBe('#123456');

    project.layers.length = 0;
    expect(PM.Library.applyLook(entry.id)).toBe(true);
    expect(project.layers).toHaveLength(1);
    expect(project.layers[0].type).toBe('shader');
    expect(project.layers[0].d.code).toContain('vec4(uTint, 1.)');
    expect(project.layers[0].d.uniforms.uTint.v).toBe('#123456');
  });

  it('recovers trashed sections and bounds project library storage at 24', () => {
    const { PM } = libraryModel();
    baseProject(PM);
    const layer = PM.mkLayer('solid', { name: 'A' }, PM.proj);
    PM.proj.layers.push(layer);
    const first = PM.Library.saveSection('one');
    PM.Library.saveSection('two');
    const layerIds = PM.proj.layers.map((item: any) => item.id);

    PM.Library.trashSection(first.id);
    expect(PM.Library.catalog('project').map((section: any) => section.name)).toEqual(['two']);
    expect(PM.Library.catalog('project', true).find((section: any) => section.id === first.id).deletedAt).toBeGreaterThan(0);
    expect(PM.proj.layers.map((item: any) => item.id)).toEqual(layerIds);
    expect(PM.__historyLabels.at(-1)).toBe('Delete section');

    PM.Library.restoreSection(first.id);
    expect(PM.Library.catalog('project').map((section: any) => section.name).sort()).toEqual(['one', 'two']);
    expect(PM.__historyLabels.at(-1)).toBe('Restore section');

    for (let index = 0; index < 30; index++) PM.Library.saveSection(`bulk ${index}`);
    expect(PM.Library.all().sections).toHaveLength(24);
  });

  it('retains an editable source reference on inserted section layers', () => {
    const { PM } = libraryModel();
    const project = baseProject(PM);
    const layer = PM.mkLayer('shape', { name: 'Editable source' }, project);
    project.layers.push(layer);
    const section = PM.Library.saveSection('Reusable', [layer.id]);
    project.layers.length = 0;

    const [inserted] = PM.Library.insertSection(section.id);

    expect(inserted.sectionRef.sectionId).toBe(section.id);
    expect(inserted.sectionRef.sourceProjectId).toBe(project.id);
    expect(typeof inserted.sectionRef.instanceId).toBe('string');
    expect(inserted.id).not.toBe(layer.id);
  });

  it('combines project-owned sections across the global library scope', () => {
    const { PM } = libraryModel();
    const first = baseProject(PM);
    first.name = 'Velocity Study';
    const firstLayer = PM.mkLayer('shape', { name: 'Hero' }, first);
    first.layers.push(firstLayer);
    PM.Library.saveSection('Hero block', [firstLayer.id]);

    const second = PM.mkProject({ name: 'Campaign', w: 1920, h: 1080, fps: 30, dur: 10 });
    const secondLayer = PM.mkLayer('text', { name: 'Title' }, second);
    second.layers.push(secondLayer);
    PM.proj = second;
    PM.Library.saveSection('Campaign title', [secondLayer.id]);
    PM.proj = first;
    const projects = new Map([[first.id, first], [second.id, second]]);
    PM.Projects = {
      list: () => [...projects.values()].map((project) => ({ id: project.id, name: project.name })),
      get: (id: string) => projects.get(id),
      put: (project: any) => projects.set(project.id, project),
    };

    const names = PM.Library.catalog('global')
      .map((section: any) => `${section.sourceProjectName}:${section.name}`)
      .sort();
    expect(names).toEqual(['Campaign:Campaign title', 'Velocity Study:Hero block']);
    expect(second.library.sections[0].layers[0].name).toBe('Title');
  });

  it('strips the retired chat panel while preserving active dock panels', () => {
    const PM = makePM('core/workspace');
    const workspace = PM.WS.normalize({
      id: 'w1',
      name: 'Legacy',
      layout: { docks: [
        { id: 'left', panels: [{ id: 'chat' }, { id: 'assets' }] },
        { id: 'center', panels: [{ id: 'viewer', flex: true }] },
      ] },
    });

    const panelIds = workspace.layout.docks.flatMap((dock: any) => dock.panels.map((panel: any) => panel.id));
    expect(panelIds).not.toContain('chat');
    expect(panelIds).toContain('assets');
  });
});
