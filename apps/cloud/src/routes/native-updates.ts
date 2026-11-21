const VERSION = /^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$/;
const CONTENT_TYPES: Record<string, string> = {
  '.xml': 'application/rss+xml; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.zip': 'application/zip',
  '.dmg': 'application/x-apple-diskimage',
};
const notFound = () => new Response(null, { status: 404, headers: { 'Cache-Control': 'no-store' } });

/** Stream the native release namespace from R2, before opening a registry DB connection. */
export async function nativeUpdateResponse(request: Request, bucket: R2Bucket): Promise<Response | null> {
  const pathname = new URL(request.url).pathname;
  if (pathname === '/native') return notFound();
  if (!pathname.startsWith('/native/')) return null;
  if (request.method !== 'GET' && request.method !== 'HEAD') {
    return new Response(null, { status: 405, headers: { Allow: 'GET, HEAD' } });
  }

  const [, , stream, filename, extra] = pathname.split('/');
  if (!stream || !filename || extra !== undefined) return notFound();
  const latest = stream === 'latest';
  let version = stream;
  const extension = filename.slice(filename.lastIndexOf('.'));
  if (latest) {
    if (filename !== 'appcast.xml' && filename !== 'policy.json') return notFound();
    const pointer = await bucket.get('native/latest/version');
    if (!pointer || pointer.size > 64) return notFound();
    version = (await pointer.text()).trim();
    if (!VERSION.test(version)) return notFound();
  } else {
    if (!VERSION.test(version) || !['.zip', '.dmg'].includes(extension)) return notFound();
    if (!['arm64', 'x64'].some(architecture => filename === `Powermove-Native-${version}-mac-${architecture}${extension}`)) {
      return notFound();
    }
  }

  const key = `native/${version}/${filename}`;
  const object = request.method === 'HEAD' ? await bucket.head(key) : await bucket.get(key);
  if (!object) return notFound();
  const headers = new Headers({
    'Content-Type': CONTENT_TYPES[extension]!,
    'Content-Length': String(object.size),
    'Cache-Control': latest ? 'no-store' : 'public, max-age=31536000, immutable',
    ETag: object.httpEtag,
    'X-Content-Type-Options': 'nosniff',
  });
  if (!latest) headers.set('Content-Disposition', `attachment; filename="${filename}"`);
  const body = request.method === 'GET' && 'body' in object ? object.body as ReadableStream : null;
  return new Response(body, { headers });
}
