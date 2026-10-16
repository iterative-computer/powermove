/* Settings › Project › Color writes (docs/rust-engine.md section 9): each
   change is one Undo step through history, so the engine's document mirror
   sees it as a patch; returning to the default removes the field. */
import { nextProjectColor, readProjectColor, type ProjectColorSettings } from '../../../shared/project-color';

export function projectColorSettings(PM: any): ProjectColorSettings {
  const color = readProjectColor(PM.proj);
  return { linearBlending: color.linearBlending, precision: color.precision };
}

export function applyProjectColor(PM: any, patch: Partial<ProjectColorSettings>): boolean {
  if (!PM.proj) return false;
  const next = nextProjectColor(PM.proj, patch);
  if (next === null) return false;
  const apply = () => {
    if (next) PM.proj.color = next;
    else delete PM.proj.color;
  };
  const label = patch.precision !== undefined ? 'Precision' : 'Blend in linear light';
  if (PM.hist?.do) PM.hist.do(label, apply);
  else apply();
  PM.touch?.();
  PM.bus?.emit?.('project');
  PM.invalidate?.('render');
  PM.autosave?.();
  return true;
}
