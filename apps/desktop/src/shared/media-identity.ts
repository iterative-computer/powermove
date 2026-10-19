/** Persisted import identity, shared by the desktop decoder and native host. */
export function assetIdentity(id: any, file: any, kind: any, prepared: any, fingerprint: any, storageKey: any, sourcePath: any, persisted: any, layerDefinition?: any) {
  return {
    id, name: file.name, kind, fingerprint, storageKey,
    ...(sourcePath ? { sourcePath } : {}),
    size: prepared.size, dur: prepared.dur, w: prepared.w, h: prepared.h,
    ...(prepared.fps ? { fps: prepared.fps } : {}),
    channels: prepared.channels || 0, sampleRate: prepared.sampleRate || 0,
    playbackProxy: prepared.playbackProxy === true,
    playbackProxyVersion: Number(prepared.playbackProxyVersion) || 0,
    persisted: persisted === true,
    ...(prepared.imageSequence ? { imageSequence: prepared.imageSequence } : {}),
    ...(prepared.poster === true ? { poster: true } : {}),
    ...(prepared.format ? { format: prepared.format } : {}),
    ...(prepared.svg ? { editablePaths: prepared.svg.paths.length } : {}),
    /* Media store (section 21): the content hash, a browser rendition kept as
       generated media, and Link to files. Absent storage means copied. */
    ...(prepared.rendition ? { rendition: true } : {}),
    ...(typeof prepared.contentHash === 'string' ? { contentHash: prepared.contentHash } : {}),
    ...(prepared.storage === 'linked' && prepared.link ? { storage: 'linked', link: prepared.link } : {}),
    ...(kind === 'video' ? {
      hasAudio: prepared.hasAudio === true,
      audioDur: prepared.hasAudio ? prepared.audioDur || prepared.dur || 0 : 0,
    } : {}),
    ...(kind === 'model' ? {
      format: prepared.format || 'obj',
      vertices: prepared.vertices || 0,
      triangles: prepared.triangles || 0,
      layerDefinition: typeof layerDefinition === 'string' ? layerDefinition : undefined,
    } : {}),
  };
}
