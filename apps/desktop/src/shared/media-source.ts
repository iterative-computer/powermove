/** An immutable media capability. Reading never retains the whole payload. */
export interface MediaReadSource {
  size: number;
  type: string;
  sha256?: string;
  read(offset: number, length: number, signal?: AbortSignal): Promise<Uint8Array>;
}
