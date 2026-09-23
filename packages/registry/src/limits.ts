export const REGISTRY_LIMITS = {
  /** Maximum regular files in a release tree; `too_many_files`. */
  files: 400,
  /** Maximum sum of uncompressed blob bytes; `tree_too_large`. */
  treeBytes: 8 * 1024 * 1024,
  /** Maximum uncompressed bytes in one blob; `file_too_large`. */
  fileBytes: 2 * 1024 * 1024,
  /** Maximum UTF-8 bytes in one relative path; `path_too_long`. */
  pathChars: 200,
  /** Maximum characters in a manifest's forkedFrom value; `manifest_invalid`. */
  forkedFromChars: 160,
} as const;

export const UPLOAD_ENVELOPE = {
  /** Maximum parts in one upload request; `too_large` with limit `envelope`. */
  partsPerRequest: 64,
  /** Maximum compressed request bytes; `too_large` with limit `envelope`. */
  compressedBytesPerRequest: 24 * 1024 * 1024,
  /** Maximum inflated request bytes; `too_large` with limit `envelope`. */
  inflatedBytesPerRequest: 48 * 1024 * 1024,
  /** Maximum simultaneous object writes; `rate_limited`. */
  concurrentWrites: 6,
  /** Maximum bytes held by orphan objects; `quota_exceeded`. */
  orphanQuotaBytes: 256 * 1024 * 1024,
  /** Maximum hours an upload lease is held; `gone` after expiry. */
  leaseHours: 24,
} as const;

export type RegistryLimitCode = 'too_many_files' | 'tree_too_large' | 'file_too_large' | 'path_too_long';
export type RegistryLimits = { [K in keyof typeof REGISTRY_LIMITS]: number };
