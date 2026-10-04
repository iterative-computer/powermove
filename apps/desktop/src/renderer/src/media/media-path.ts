/*
 * Renderer seam: turn a project media asset into an absolute file path the
 * main process (or the remote host) can read with ffmpeg or the
 * transcription engine. The media-tools lane owns the implementation:
 * the original source path when it still exists, otherwise the asset's bytes
 * staged once to a cache file (desktop) or uploaded to the host (web bridge).
 *
 * Rejects when the asset does not exist or has no audio/video bytes.
 */
export async function resolveMediaPath(assetId: string): Promise<string> {
  throw new Error(`resolveMediaPath is not implemented yet (asset ${assetId}).`);
}
