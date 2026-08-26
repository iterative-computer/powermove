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
  mods: () => import('../../../extensions/mods/index') as Promise<ExtensionModule>,
};
