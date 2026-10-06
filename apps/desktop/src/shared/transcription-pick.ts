/*
 * Which downloaded model a request runs on. Main (ModelStore.modelFor) and
 * the renderer (format.ts modelFor) both call this, so the download sheet,
 * the Settings note and the engine always agree on the pick.
 */

export interface PickableModel {
  id: string;
  languages: 'en' | 'multi';
  languageCodes?: readonly string[];
  wordTimestamps: boolean;
  /** False when the model must be told the language; it then hears English. */
  detectsLanguage?: boolean;
}

/** 'de' for 'de-AT'; '' for 'auto' or nothing. */
export function baseLanguage(tag: string | undefined): string {
  return tag && tag !== 'auto' ? tag.split('-')[0]!.toLowerCase() : '';
}

const codesOf = (model: PickableModel): readonly string[] =>
  model.languageCodes ?? (model.languages === 'en' ? ['en'] : []);

/**
 * The language a request asks for, as the model in use would hear it: the
 * tag, or English for 'auto' when that model cannot detect (or only knows
 * English). '' when nobody knows: a model that detects, on 'auto'.
 */
export function spokenLanguage(active: PickableModel | null, language: string | undefined): string {
  const base = baseLanguage(language);
  if (base || !active) return base;
  return active.languages === 'en' || active.detectsLanguage === false ? 'en' : '';
}

/**
 * The model a request runs on: the active one, unless the request needs
 * word timing it lacks. Then a downloaded word-timed model that can hear the
 * same speech: one listing the spoken language; with the language unknown
 * and a multilingual model in use, one that detects the language too, never
 * an English-only model (it would turn other speech into nonsense). Null
 * when nothing downloaded fits, so callers offer the download sheet.
 */
export function pickModel<T extends PickableModel>(
  active: T | null,
  ready: readonly T[],
  needs: { wordTimestamps?: boolean; language?: string } = {}
): T | null {
  if (!needs.wordTimestamps || active?.wordTimestamps) return active;
  const timed = ready.filter((model) => model.wordTimestamps);
  const spoken = spokenLanguage(active, needs.language);
  if (spoken) return timed.find((model) => codesOf(model).includes(spoken)) ?? null;
  if (active?.languages === 'multi') return timed.find((model) => model.languages === 'multi' && model.detectsLanguage !== false) ?? null;
  return timed[0] ?? null;
}
