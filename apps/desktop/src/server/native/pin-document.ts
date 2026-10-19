/** Serialized into the DOM tab by Playwright. Keep this function self-contained. */
export async function pinNativeDocument(doc: Record<string, any>): Promise<void> {
  const PM = (globalThis as any).PM;
  if (!await PM.openProjectHere(doc.id)) throw new Error('The pinned project could not be opened.');
  await PM.EngineDoc?.whenReady;
  await PM.RemoteSync.refresh();
  // Opening the active ID is a no-op. Its boot copy may predate the worker,
  // including recovered journal edits. A newer joined revision still wins.
  if (Number(PM.proj.revision ?? 0) <= Number(doc.revision ?? 0)) PM.replaceProject(doc, { selection: PM.sel });
  await PM.RemoteSync.flush();
}
