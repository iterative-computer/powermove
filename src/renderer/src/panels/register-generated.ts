import { mount, unmount } from 'svelte';
import type { GeneratedSection, WorkspaceManifest } from '../core/types/workspace';
import GeneratedPanel from './GeneratedPanel.svelte';

type LegacyPM = Record<string, any>;
type Mounted = { instance: object; body: HTMLElement };

/* Generated panels rebuild on activate/mutate, unlike long-lived builtin
   Svelte panels. A dedicated mount map gives this adapter targeted unmounts;
   registerSveltePanel's private global map can only unmount all panel kinds. */
const mounts = new WeakMap<LegacyPM, Map<string, Mounted>>();
const generatedIds = new WeakMap<LegacyPM, Set<string>>();

function mountMap(PM: LegacyPM): Map<string, Mounted> {
  let current = mounts.get(PM);
  if (!current) {
    current = new Map();
    mounts.set(PM, current);
  }
  return current;
}

function dispose(PM: LegacyPM, id: string): void {
  const current = mountMap(PM);
  const mounted = current.get(id);
  if (mounted) {
    void unmount(mounted.instance);
    current.delete(id);
  }
  // The panel pool caches persistent panels by this entry. Removing it makes
  // the next apply rebuild the panel with the new section manifest.
  if (PM.panelInst) delete PM.panelInst[id];
}

function registerGeneratedPanel(PM: LegacyPM, section: GeneratedSection): void {
  PM.registerPanel(section.id, {
    title: section.title || 'Panel',
    size: section.size || 200,
    persist: true,
    build(body: HTMLElement, inst: Record<string, any>) {
      const current = mountMap(PM);
      const prior = current.get(section.id);
      if (prior) void unmount(prior.instance);
      body.replaceChildren();
      const instance = mount(GeneratedPanel, {
        target: body,
        props: {
          panelId: section.id,
          spec: inst.spec ?? {},
          PM,
          section
        }
      });
      current.set(section.id, { instance, body });
    }
  });
}

/** Register every manifest-authored section and invalidate prior custom mounts. */
export function registerGeneratedPanels(
  PM: LegacyPM,
  workspace: Pick<WorkspaceManifest, 'custom'> | { custom?: GeneratedSection[] }
): void {
  const sections = Array.isArray(workspace.custom) ? workspace.custom : [];
  const nextIds = new Set(sections.map((section) => section.id));
  for (const id of generatedIds.get(PM) || []) {
    if (!nextIds.has(id)) dispose(PM, id);
  }
  for (const section of sections) {
    dispose(PM, section.id);
    registerGeneratedPanel(PM, section);
  }
  generatedIds.set(PM, nextIds);
}

/** Integration seam installed after the workspace engine. */
export function installGeneratedPanels(PM: LegacyPM): void {
  PM.WS.registerCustom = (workspace: Pick<WorkspaceManifest, 'custom'>) =>
    registerGeneratedPanels(PM, workspace);
}

/** Test/HMR seam, deliberately scoped to generated panels. */
export function unmountGeneratedPanels(PM: LegacyPM): void {
  for (const id of generatedIds.get(PM) || []) dispose(PM, id);
  generatedIds.delete(PM);
  mounts.delete(PM);
}
