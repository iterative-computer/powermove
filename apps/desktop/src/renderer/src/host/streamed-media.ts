/**
 * A media file the editor reads from disk on demand instead of copying it
 * into a Blob.
 *
 * Restoring a project used to fetch every clip in full (`response.blob()`)
 * and play it from a `blob:` URL. Chromium keeps blob bytes in the browser
 * process, so a project with 800 MB of footage held 800 MB of copies for as
 * long as it stayed open. Video elements can instead play the library URL
 * directly, which streams ranges from disk and shares one cache per file.
 *
 * Code that needs bytes (container probes, embedded audio decode, export
 * demuxing) only uses `size`, `type`, `slice(...).arrayBuffer()` and
 * `arrayBuffer()`; this class answers those with range requests.
 */
const MAX_BOUNDED_RANGE = 1024 * 1024; // The library's limit for `bytes=a-b`.

export class StreamedMedia {
  constructor(
    readonly url: string,
    readonly size: number,
    readonly type = '',
    private readonly offset = 0,
    private readonly total = size,
  ) {}

  /** Same clamping rules as Blob.slice, including negative offsets. */
  slice(start = 0, end = this.size, type = ''): StreamedMedia {
    const clamp = (value: number) => {
      const at = Number.isFinite(value) ? Math.trunc(value) : 0;
      return at < 0 ? Math.max(0, this.size + at) : Math.min(this.size, at);
    };
    const from = clamp(start), to = Math.max(from, clamp(end));
    return new StreamedMedia(this.url, to - from, type, this.offset + from, this.total);
  }

  async arrayBuffer(): Promise<ArrayBuffer> {
    if (!this.size) return new ArrayBuffer(0);
    const toEnd = this.offset + this.size >= this.total;
    // A whole file is one plain request; a tail is one open-ended range.
    if (toEnd) return this.fetchRange(this.offset === 0 ? null : `bytes=${this.offset}-`, this.size);
    if (this.size <= MAX_BOUNDED_RANGE) return this.fetchRange(`bytes=${this.offset}-${this.offset + this.size - 1}`, this.size);
    const out = new Uint8Array(this.size);
    for (let at = 0; at < this.size; at += MAX_BOUNDED_RANGE) {
      const length = Math.min(MAX_BOUNDED_RANGE, this.size - at), start = this.offset + at;
      out.set(new Uint8Array(await this.fetchRange(`bytes=${start}-${start + length - 1}`, length)), at);
    }
    return out.buffer;
  }

  /** A real Blob for APIs that require one (format conversion, upload). */
  async blob(): Promise<Blob> {
    return new Blob([await this.arrayBuffer()], { type: this.type });
  }

  async text(): Promise<string> {
    return new TextDecoder().decode(await this.arrayBuffer());
  }

  private async fetchRange(range: string | null, expected: number): Promise<ArrayBuffer> {
    const response = await fetch(this.url, { cache: 'no-store', ...(range ? { headers: { Range: range } } : {}) });
    if (!response.ok) throw new Error('The project media could not be read from your library.');
    const bytes = await response.arrayBuffer();
    if (bytes.byteLength !== expected) throw new Error('The project media is incomplete.');
    return bytes;
  }
}

export function isStreamedMedia(value: unknown): value is StreamedMedia {
  return value instanceof StreamedMedia;
}

/** Plays from the library only when the page may read the pixels: a video
 * from another origin would taint WebGL uploads (the dev server is http:). */
export function canStreamFrom(url: string): boolean {
  if (typeof location === 'undefined' || !location.origin || location.origin === 'null') return false;
  return url.startsWith(location.origin + '/');
}
