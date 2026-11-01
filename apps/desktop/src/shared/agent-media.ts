/* Local media an agent shows in its reply.

   Agents write screenshots, renders and recordings to disk and then name
   them in prose: `![before](/tmp/before.png)`, a bare path, or a path in
   backticks. The panel turns those into real images, videos and audio
   players. Files load through app://powermove/_agent-file, which main serves
   only for these media extensions. */

export type AgentMediaKind = 'image' | 'video' | 'audio';

const KINDS: Readonly<Record<string, AgentMediaKind>> = {
  png: 'image', jpg: 'image', jpeg: 'image', gif: 'image', webp: 'image', avif: 'image', svg: 'image', bmp: 'image',
  mp4: 'video', m4v: 'video', mov: 'video', webm: 'video',
  mp3: 'audio', m4a: 'audio', wav: 'audio', aac: 'audio', ogg: 'audio', flac: 'audio'
};

export const AGENT_MEDIA_TYPES: Readonly<Record<string, string>> = {
  png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp', avif: 'image/avif',
  svg: 'image/svg+xml', bmp: 'image/bmp',
  mp4: 'video/mp4', m4v: 'video/mp4', mov: 'video/quicktime', webm: 'video/webm',
  mp3: 'audio/mpeg', m4a: 'audio/mp4', wav: 'audio/wav', aac: 'audio/aac', ogg: 'audio/ogg', flac: 'audio/flac'
};

export const AGENT_FILE_PREFIX = 'app://powermove/_agent-file';

export function agentMediaExtension(file: string): string {
  return /\.([a-z0-9]+)$/i.exec(file.split(/[?#]/)[0]!)?.[1]?.toLowerCase() ?? '';
}

export function agentMediaKind(file: string): AgentMediaKind | null {
  return KINDS[agentMediaExtension(file)] ?? null;
}

/** An absolute local path (also `file://` and `~/`), or null. */
export function localMediaPath(src: string): string | null {
  let value = src.trim();
  if (/^file:\/\//i.test(value)) {
    try { value = decodeURIComponent(new URL(value).pathname); } catch { return null; }
  }
  if (!value.startsWith('/') && !value.startsWith('~/')) return null;
  if (value.split('/').includes('..')) return null;
  return agentMediaKind(value) ? value : null;
}

/** The URL the panel loads a local media file from. */
export function agentFileUrl(file: string): string {
  return `${AGENT_FILE_PREFIX}?path=${encodeURIComponent(file)}`;
}
