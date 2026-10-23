/** Today's media-store filename fallback, shared with the host project packer. */
const MIME: Record<string, string> = {
  mp4: 'video/mp4', m4v: 'video/mp4', mov: 'video/quicktime', webm: 'video/webm', mkv: 'video/x-matroska',
  png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp', svg: 'image/svg+xml',
  wav: 'audio/wav', mp3: 'audio/mpeg', m4a: 'audio/mp4', aac: 'audio/aac', ogg: 'audio/ogg', flac: 'audio/flac'
};
export function mimeOf(name: unknown): string {
  const match = /\.([A-Za-z0-9]{1,8})$/.exec(String(name || ''));
  return match ? MIME[match[1]!.toLowerCase()] ?? '' : '';
}
