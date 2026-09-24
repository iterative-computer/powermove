/** IDs that would overlap host event words or shipped extension namespaces. */
export const RESERVED_STORE_IDS = new Set([
  'project', 'theme', 'extension', 'extensions', 'frame', 'inspector', 'ext',
  'powermove', 'kernel', 'host', 'app', 'text', 'timeline', '3d', 'viewer',
  'toolbar', 'mods', 'keymap', 'effects-basic', 'keymap-default', 'layers-3d',
  'theme-default', 'transitions-basic'
]);
