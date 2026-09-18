/* Settings is a full-screen surface under the titlebar, laid out like the
   Projects home: a sidebar on the window colour and one inset sheet for the
   page. The screen is Svelte (SettingsScreen); this installer keeps the
   PM.SettingsUI contract that the titlebar gear, ⌘, and the agent panel use,
   and defers any DOM work until Settings is first opened. */
import { mount } from 'svelte';

import type { PMRegistry } from '../legacy/registry';
import { viewerService } from '../legacy/core/services';
import type { CompositionPatch, ProjectSettingsBridge } from '../legacy/ui/project-settings';
import SettingsScreen, { type SettingsPage } from './SettingsScreen.svelte';

function projectSettingsBridge(PM: PMRegistry): ProjectSettingsBridge {
  return {
    composition: () => ({
      name: PM.proj.name, w: PM.proj.w, h: PM.proj.h, fps: PM.proj.fps, dur: PM.proj.dur
    }),
    applyComposition: (patch: CompositionPatch) => {
      /* Renaming goes through the project registry so the titlebar and the
         stored project slot stay in step, exactly like an inline tab rename. */
      if (patch.name != null) { PM.Projects.rename(PM.proj.id, patch.name); return; }
      try {
        PM.Edit.apply({ type: 'set_composition', patch }, { label: 'Project settings', origin: 'interface' });
      } catch (error: any) {
        PM.toast('Could not change the project: ' + (error?.message || 'invalid value'), 4500);
        return;
      }
      if (PM.time > PM.proj.dur) PM.setTime(PM.proj.dur);
      PM.rasterClear?.();
      viewerService(PM)?.layout();
      PM.invalidate('all');
    },
    exportDefaults: () => PM.exportDefaults.read(),
    applyExportDefaults: (patch) => PM.exportDefaults.write(patch),
    backgroundField: () => PM.fillField(
      () => PM.normalizeFill(PM.proj.backgroundFill, PM.proj.bg),
      (value: any) => {
        PM.proj.backgroundFill = PM.normalizeFill(value, PM.proj.bg);
        PM.proj.bg = PM.proj.backgroundFill.stops[0].color;
      },
      { label: 'Background', command: (value: any) => ({ type: 'set_composition', patch: { backgroundFill: value } }) }
    )
  };
}

export function install(PM: PMRegistry): void {
  let screen: any = null;
  function ensure() {
    if (!screen) {
      screen = mount(SettingsScreen, {
        target: document.body,
        props: {
          PM,
          /* The Project page only makes sense inside the editor. On the home
             screen PM.proj is the placeholder home project, so the page is
             left out entirely rather than editing a project nobody sees. */
          projectBridge: () => (PM.proj && !PM.ProjectsScreen?.isOpen ? projectSettingsBridge(PM) : null)
        }
      });
    }
    return screen;
  }
  PM.SettingsUI = {
    open: (page?: SettingsPage) => ensure().open(page),
    close: () => screen?.close(),
    get isOpen() { return !!screen?.isOpen(); }
  };
}
