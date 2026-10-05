import type { TranscriptionModelInfo, TranscriptionStatus } from '../../../shared/transcription';

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

/** "English only", or how many languages a model transcribes. */
export function reach(model: TranscriptionModelInfo): string {
  if (model.languages === 'en') return 'English only';
  const count = model.languageCodes?.length ?? 0;
  return count > 1 ? `${count} languages` : 'Many languages';
}

/** The facts line under a model: size, languages, and when it cannot make captions. */
export function modelFacts(model: TranscriptionModelInfo): string {
  return [formatBytes(model.size), reach(model), ...(model.wordTimestamps ? [] : ['Not for captions'])].join(' · ');
}

/** What a request needs from a model. Captions need every word timed. */
export interface ModelNeeds { wordTimestamps?: boolean }

const baseLanguage = (tag: string | undefined): string =>
  tag && tag !== 'auto' ? tag.split('-')[0]!.toLowerCase() : '';

/**
 * The model a request would run on, as main picks it (ModelStore.modelFor):
 * the active one, unless the request needs word timing it lacks; then a
 * downloaded word-timed model, one listing the language first.
 */
export function modelFor(status: TranscriptionStatus | null | undefined, needs: ModelNeeds = {}): TranscriptionModelInfo | null {
  const models = status?.models ?? [];
  const active = models.find((model) => model.id === status?.activeModelId && model.state === 'ready') ?? null;
  if (!needs.wordTimestamps || active?.wordTimestamps) return active;
  const timed = models.filter((model) => model.wordTimestamps && model.state === 'ready');
  const base = baseLanguage(status?.language);
  return (base ? timed.find((model) => model.languageCodes?.includes(base)) : undefined) ?? timed[0] ?? null;
}

/**
 * The short list the download sheet and onboarding offer: the curated
 * models (Settings has the rest), plus one already downloading or failed so
 * its progress or error stays in view. Captions see only word-timed models.
 */
export function curatedModels(models: readonly TranscriptionModelInfo[], needs: ModelNeeds = {}): TranscriptionModelInfo[] {
  const fits = (model: TranscriptionModelInfo) => !needs.wordTimestamps || model.wordTimestamps;
  const featured = models.filter((model) => fits(model) && (model.featured || model.state === 'downloading'));
  /* A catalog without curation flags (an older main) offers everything. */
  return featured.length ? featured : models.filter(fits);
}

/** Every language some model transcribes, by name, for the Settings filter. */
export function catalogLanguages(models: readonly TranscriptionModelInfo[]): Array<{ code: string; name: string }> {
  const codes = new Set(models.flatMap((model) => model.languageCodes ?? (model.languages === 'en' ? ['en'] : [])));
  return [...codes].map((code) => ({ code, name: languageName(code) })).sort((a, b) => a.name.localeCompare(b.name));
}

/** Models that transcribe a language ('all' keeps every model). */
export function modelsFor<T extends TranscriptionModelInfo>(models: readonly T[], language: string): T[] {
  if (language === 'all') return [...models];
  return models.filter((model) => (model.languageCodes ?? (model.languages === 'en' ? ['en'] : [])).includes(language));
}
