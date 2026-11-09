import { groupAncestors } from './layer-groups';

/** Projects saved before each layer owned its 3D switch drew every member of
 * a 3D group in 3D, whatever the member's own switch said. Opening one turns
 * those members' switches on, so it looks exactly as it did and the switch
 * then tells the truth. Version 2 projects are left alone. */
export const SWITCH_3D_VERSION = 2;
export function migrateInherited3D(project: any): any {
  if (!project || typeof project !== 'object' || typeof project.switch3DVersion === 'number' && project.switch3DVersion >= SWITCH_3D_VERSION) return project;
  const migrate = (container: any) => {
    const layers: any[] = Array.isArray(container?.layers) ? container.layers : [];
    for (const layer of layers) {
      if (!layer || layer.threeD === true || layer.type === 'audio' || layer.type === 'adjustment' || layer.effectSpace === 'source') continue;
      if (groupAncestors(layer, layers).some(group => group.threeD === true)) layer.threeD = true;
    }
  };
  migrate(project);
  for (const comp of Object.values<any>(project.comps || {})) migrate(comp);
  for (const section of project.library?.sections || []) {
    migrate(section);
    for (const version of section.versions || []) migrate(version);
  }
  project.switch3DVersion = SWITCH_3D_VERSION;
  return project;
}
