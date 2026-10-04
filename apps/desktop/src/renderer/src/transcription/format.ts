import type { TranscriptionModelInfo } from '../../../shared/transcription';

/* Words and numbers the transcription UI shows, in one place. */

export function formatBytes(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${Math.round(bytes / 1e6)} MB`;
  if (bytes >= 1e3) return `${Math.round(bytes / 1e3)} KB`;
  return bytes > 0 ? `${bytes} bytes` : 'Nothing';
}

const names = (() => {
  try { return new Intl.DisplayNames(['en'], { type: 'language' }); } catch { return null; }
})();

export function languageName(code: string): string {
  try { return names?.of(code) ?? code; } catch { return code; }
}

/** The active model's languages as picker options, by name. */
export function languageOptions(model: TranscriptionModelInfo | null | undefined): Array<{ code: string; name: string }> {
  return (model?.languageCodes ?? [])
    .map((code) => ({ code, name: languageName(code) }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

/** "312 MB of 670 MB" while downloading. */
export function downloadLine(model: TranscriptionModelInfo): string {
  const done = model.downloadedBytes ?? Math.round((model.progress ?? 0) * model.size);
  return `${formatBytes(done)} of ${formatBytes(model.size)}`;
}

export function percent(progress: number | undefined): string {
  return `${Math.floor(Math.max(0, Math.min(1, progress ?? 0)) * 100)}%`;
}
