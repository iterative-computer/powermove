import type { PMRegistry } from '../registry';

export function requiredText(value: unknown, name: string): string {
  if (typeof value !== 'string' || !value.trim() || value.length > 4096) throw new Error(`${name} is required.`);
  return value.trim();
}

export function finite(value: unknown, name: string, min: number, max: number): number {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < min || value > max) throw new Error(`Invalid ${name}: expected ${min}–${max}.`);
  return value;
}

/** Library operations persist immediately and are deliberately outside timeline Undo. */
export async function manageProject(PM: PMRegistry, args: Record<string, any>) {
  const operation = args.operation;
  const id = args.projectId ?? PM.proj.id;
  const name = () => requiredText(args.name, 'name').slice(0, 240);
  if (operation === 'list') return {
    activeProjectId: PM.proj.id,
    projects: PM.Projects.list().map(({ id, name, w, h, at }: any) => ({ id, name, w, h, at })),
    trash: PM.Projects.trashList().map(({ id, name }: any) => ({ id, name })),
  };
  if (operation === 'create') {
    const options: Record<string, any> = { name: name() };
    for (const [key, min, max] of [['w', 2, 8192], ['h', 2, 8192], ['fps', 1, 240], ['dur', .01, 86400]] as const) {
      if (args[key] !== undefined) options[key] = finite(args[key], key, min, max);
    }
    if (['w', 'h'].some(key => options[key] !== undefined && !Number.isInteger(options[key]))) throw new Error('Dimensions must be integers.');
    const project = PM.mkProject(options);
    PM.Projects.put(project);
    if (!await PM.openProjectHere(project.id)) throw new Error('Project created but could not be opened. Use manage_project list to inspect it.');
    await PM.projectAssetsReady;
    PM.ProjectsScreen?.hide?.();
    await PM.store.flush?.();
    return { projectId: project.id, name: project.name, active: true };
  }
  requiredText(id, 'projectId');
  if (operation === 'restore') {
    if (!PM.Projects.restore(id)) throw new Error('Could not restore this project.');
  } else {
    const project = id === PM.proj.id ? PM.proj : PM.Projects.get(id);
    if (!project) throw new Error('Project not found.');
    if (operation === 'open') {
      if (!await PM.openProjectHere(id)) throw new Error('Could not open project here; it may be owned by another window.');
      await PM.projectAssetsReady;
      PM.ProjectsScreen?.hide?.();
    } else if (operation === 'save') {
      PM.bus.emit('project:flush-edits');
      await PM.app?.importQueue;
      PM.Projects.put(project);
    } else if (operation === 'rename') {
      if ((PM.Projects.openProjects?.() || []).includes(id) && id !== PM.proj.id) throw new Error('Rename this project in its owning window.');
      PM.Projects.rename(id, name());
    } else if (operation === 'duplicate') {
      if ((PM.Projects.openProjects?.() || []).includes(id) && id !== PM.proj.id) throw new Error('Duplicate this project in its owning window to include unsaved edits.');
      const copy = JSON.parse(JSON.stringify(project));
      copy.id = PM.uid('P'); copy.name = name(); copy.revision = 0; copy.edits = []; copy.created = Date.now();
      PM.Projects.put(copy);
      await PM.store.flush?.();
      return { projectId: copy.id, name: copy.name, active: false };
    } else if (operation === 'trash') {
      if (id === PM.proj.id || (PM.Projects.openProjects?.() || []).includes(id)) throw new Error('Close the project before moving it to trash.');
      if (!PM.Projects.trash(id)) throw new Error('Could not trash project.');
    } else throw new Error('Unknown project operation.');
  }
  await PM.store.flush?.();
  return { projectId: id, operation, activeProjectId: PM.proj.id };
}
