/*
 * Built-in extension bundle table.
 *
 * The loader boots these before any user/project extension, in the order the
 * keys appear here. Each value is a lazy `import()` so a built-in that fails to
 * parse takes down only itself.
 *
 * Built-in editor surfaces live in `src/extensions/<id>/` and use the same
 * activation path as user/project extensions. Keep the foundational theme,
 * keymap and render definitions ahead of UI surfaces that consume them.
 */
import type { ExtensionModule } from './api';

export const BUILTIN_EXTENSIONS: Record<string, () => Promise<ExtensionModule>> = {
  'theme-default': () => import('../../../extensions/theme-default/index') as Promise<ExtensionModule>,
  'keymap-default': () => import('../../../extensions/keymap-default/index') as Promise<ExtensionModule>,
  'effects-basic': () => import('../../../extensions/effects-basic/index') as Promise<ExtensionModule>,
  'transitions-basic': () => import('../../../extensions/transitions-basic/index') as Promise<ExtensionModule>,
  toolbar: () => import('../../../extensions/toolbar/index') as Promise<ExtensionModule>,
  viewer: () => import('../../../extensions/viewer/index') as Promise<ExtensionModule>,
  timeline: () => import('../../../extensions/timeline/index') as Promise<ExtensionModule>,
  inspector: () => import('../../../extensions/inspector/index') as Promise<ExtensionModule>,
  mods: () => import('../../../extensions/mods/index') as Promise<ExtensionModule>,
};

// Stop built-in panel edits at the extension boundary instead of letting Vite
// bubble them into a full document reload. The loader preserves panel placement.
if (import.meta.hot) {
  const ids = ['theme-default', 'keymap-default', 'effects-basic', 'transitions-basic', 'toolbar', 'viewer', 'timeline', 'inspector', 'mods'];
  import.meta.hot.accept([
    '../../../extensions/theme-default/index',
    '../../../extensions/keymap-default/index',
    '../../../extensions/effects-basic/index',
    '../../../extensions/transitions-basic/index',
    '../../../extensions/toolbar/index',
    '../../../extensions/viewer/index',
    '../../../extensions/timeline/index',
    '../../../extensions/inspector/index',
    '../../../extensions/mods/index'
  ], async (modules) => {
    for (let index = 0; index < modules.length; index++) {
      const module = modules[index], id = ids[index];
      if (!module || !id || typeof module.default !== 'function') continue;
      BUILTIN_EXTENSIONS[id] = () => Promise.resolve(module as unknown as ExtensionModule);
      try { await window.PM?.Kernel?.loader?.reload(id); }
      catch (error) { console.error('[panel hot update]', error); window.PM?.toast?.('Panel update failed. Your editing session is still open.'); }
    }
  });
}
