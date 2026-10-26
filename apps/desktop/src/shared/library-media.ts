export const LIBRARY_MEDIA_IPC = {
  begin: 'library-media:begin', chunk: 'library-media:chunk', finish: 'library-media:finish',
  abort: 'library-media:abort', get: 'library-media:get', remove: 'library-media:remove', link: 'library-media:link'
} as const;
export const LIBRARY_MEDIA_CHUNK = 1024 * 1024;
export type ImportedArtifact = { projectId: string; path: string };
export type LibraryMediaInput = { key: string; size: number; type: string; revision?: string; onlyIfMissing?: boolean; artifact?: ImportedArtifact };
export type LibraryMediaLink = { key: string; sourcePath: string; size: number; type: string; revision?: string; matchExisting?: boolean };
export type LibraryMediaRecord = { revision: string; size: number; type: string; url: string; sha256?: string; sourcePath?: string };
export interface LibraryMediaBridge {
  begin(input: LibraryMediaInput): Promise<string>;
  chunk(token: string, offset: number, data: Uint8Array): Promise<void>;
  finish(token: string): Promise<LibraryMediaRecord>;
  abort(token: string): Promise<void>;
  get(key: string, metadataOnly?: boolean): Promise<LibraryMediaRecord | null>;
  remove(key: string): Promise<boolean>;
  link?(input: LibraryMediaLink): Promise<LibraryMediaRecord>;
}
