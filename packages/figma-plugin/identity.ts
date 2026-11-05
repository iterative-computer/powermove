/** Document-owned identity works in Community and local plugins alike. It never
 * asks for private file APIs and is stable across plugin and app restarts. */
export function figmaFileId(root: {
  getPluginData(key: string): string;
  setPluginData(key: string, value: string): void;
}): string {
  const key = 'powermove-file-id';
  let id = root.getPluginData(key);
  if (!id) {
    id =
      'pm-' +
      Date.now().toString(36) +
      '-' +
      Array.from({ length: 4 }, () => Math.random().toString(36).slice(2)).join('');
    root.setPluginData(key, id);
  }
  return id;
}
