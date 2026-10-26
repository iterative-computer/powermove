/** NSFileVersion is an optional local convenience. Private recovery remains authoritative. */
type Native = typeof import('@powermove/macos-haptics');
let addon: Promise<Native | null> | undefined;
function native(): Promise<Native | null> {
  return addon ??= process.platform === 'darwin' ? import('@powermove/macos-haptics').then(module =>
    (module as Native & { default?: Native }).default ?? module).catch(() => null) : Promise.resolve(null);
}
export async function snapshotNativeVersion(destination: string, contents: string): Promise<boolean> {
  return (await native())?.fileVersions?.('snapshot', destination, contents).then(result => result !== null) ?? false;
}
export async function listNativeVersions(destination: string): Promise<Array<{ path: string; at: number }>> {
  const result = await (await native())?.fileVersions?.('list', destination);
  if (!result) return [];
  const parsed: unknown = JSON.parse(result);
  if (!Array.isArray(parsed)) throw new Error('Invalid native version list');
  return parsed.filter((v): v is { path: string; at: number } => v && typeof v.path === 'string' && Number.isFinite(v.at));
}
