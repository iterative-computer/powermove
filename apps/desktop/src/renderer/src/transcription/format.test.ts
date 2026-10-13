import { describe, expect, it } from 'vitest';

import type { TranscriptionModelInfo, TranscriptionStatus } from '../../../shared/transcription';
import { captionGap, captionGapLine, captionSuggestion, catalogLanguages, curatedModels, modelFacts, modelFor, modelsFor, reach, sheetModels } from './format';

const model = (id: string, extra: Partial<TranscriptionModelInfo> = {}): TranscriptionModelInfo => ({
  id, name: id, description: '', size: 731_357_568, languages: 'multi', languageCodes: ['en', 'de', 'fr'],
  speed: 0.8, accuracy: 0.8, recommended: false, wordTimestamps: true, state: 'absent', ...extra
});

const unified = model('unified', { languages: 'en', languageCodes: ['en'], detectsLanguage: false, recommended: true, featured: true });
const nemotron = model('nemotron', { languageCodes: ['en', 'de', 'ja'], detectsLanguage: true, recommended: true, featured: true });
const v3 = model('v3', { languageCodes: ['en', 'de', 'fr'], detectsLanguage: true, featured: true });
const whisper = model('whisper', { wordTimestamps: false, timing: 'segment', languageCodes: ['en', 'de', 'fr', 'ja', 'sw'] });
const cohere = model('cohere', { wordTimestamps: false, timing: 'none', detectsLanguage: false, size: 1_770_270_208 });

describe('model facts', () => {
  it('names the reach, size and, only when it matters, that a model cannot make captions', () => {
    expect(reach(unified)).toBe('English only');
    expect(reach(nemotron)).toBe('3 languages');
    expect(modelFacts(unified)).toBe('731 MB · English only');
    expect(modelFacts(cohere)).toBe('1.8 GB · 3 languages · Not for captions');
  });
});

describe('modelFor', () => {
  const status = (models: TranscriptionModelInfo[], activeModelId: string | null, language = 'auto'): TranscriptionStatus => ({ models, activeModelId, language });

  it('is the active model, unless captions need word timing it lacks', () => {
    const ready = (entry: TranscriptionModelInfo) => ({ ...entry, state: 'ready' as const });
    expect(modelFor(status([ready(whisper)], 'whisper'))?.id).toBe('whisper');
    expect(modelFor(status([ready(whisper)], 'whisper'), { wordTimestamps: true })).toBeNull();
    // A downloaded word-timed model stands in, one listing the spoken language first.
    expect(modelFor(status([ready(unified), ready(nemotron), ready(whisper)], 'whisper', 'ja'), { wordTimestamps: true })?.id).toBe('nemotron');
    // Whisper on automatic: a multilingual stand-in, never the English-only one listed first.
    expect(modelFor(status([ready(unified), ready(nemotron), ready(whisper)], 'whisper'), { wordTimestamps: true })?.id).toBe('nemotron');
    expect(modelFor(status([ready(unified), ready(whisper)], 'whisper'), { wordTimestamps: true })).toBeNull();
    expect(modelFor(status([ready(unified), ready(whisper)], 'whisper', 'en'), { wordTimestamps: true })?.id).toBe('unified');
    // A model that cannot detect hears English, so Unified EN fits.
    expect(modelFor(status([ready(unified), ready(cohere)], 'cohere'), { wordTimestamps: true })?.id).toBe('unified');
    // Downloading is not ready.
    expect(modelFor(status([{ ...unified, state: 'downloading' }, ready(whisper)], 'whisper'), { wordTimestamps: true })).toBeNull();
    expect(modelFor(status([], null))).toBeNull();
  });
});

describe('when captions cannot run on what is downloaded', () => {
  const ready = (entry: TranscriptionModelInfo) => ({ ...entry, state: 'ready' as const });
  const status = (models: TranscriptionModelInfo[], activeModelId: string | null, language = 'auto'): TranscriptionStatus => ({ models, activeModelId, language });

  it('says why, and offers only models that would let captions run', () => {
    const englishOnly = status([ready(unified), nemotron, v3, ready(whisper)], 'whisper');
    const gap = captionGap(englishOnly)!;
    expect(captionGapLine(gap)).toBe('whisper doesn’t time each word, which captions need, and unified transcribes only English.');
    expect(captionSuggestion(englishOnly)?.id).toBe('nemotron');
    expect(sheetModels(englishOnly, { wordTimestamps: true }).map((entry) => entry.id)).toEqual(['nemotron', 'v3']);
    // A known language the downloaded word-timed model does not hear.
    const japanese = status([ready(unified), nemotron, v3, ready(whisper)], 'whisper', 'ja');
    expect(captionGapLine(captionGap(japanese)!)).toBe('whisper doesn’t time each word, which captions need, and unified doesn’t transcribe Japanese.');
    expect(sheetModels(japanese, { wordTimestamps: true }).map((entry) => entry.id)).toEqual(['nemotron']);
    // Nothing word-timed yet: the plain reason, and every curated word-timed model would do for a model that cannot detect.
    const untimed = status([unified, nemotron, v3, ready(cohere)], 'cohere');
    expect(captionGapLine(captionGap(untimed)!)).toBe('cohere doesn’t time each word, which captions need.');
    expect(sheetModels(untimed, { wordTimestamps: true }).map((entry) => entry.id)).toEqual(['unified', 'nemotron', 'v3']);
    expect(captionSuggestion(untimed)?.id).toBe('unified');
    // Captions can run: no gap.
    expect(captionGap(status([ready(nemotron), ready(whisper)], 'whisper'))).toBeNull();
  });
});

describe('curatedModels', () => {
  it('offers the featured models plus one in flight, and only word-timed ones for captions', () => {
    const busy = { ...whisper, state: 'downloading' as const };
    expect(curatedModels([unified, nemotron, whisper, cohere]).map((entry) => entry.id)).toEqual(['unified', 'nemotron']);
    expect(curatedModels([unified, nemotron, busy, cohere]).map((entry) => entry.id)).toEqual(['unified', 'nemotron', 'whisper']);
    expect(curatedModels([unified, nemotron, busy, cohere], { wordTimestamps: true }).map((entry) => entry.id)).toEqual(['unified', 'nemotron']);
    // Without curation flags, everything that fits.
    expect(curatedModels([whisper, model('plain')], { wordTimestamps: true }).map((entry) => entry.id)).toEqual(['plain']);
  });
});

describe('the language filter', () => {
  it('lists every language by name and keeps the models that transcribe one', () => {
    const languages = catalogLanguages([unified, nemotron, whisper]);
    expect(languages.map((entry) => entry.code)).toEqual(['en', 'fr', 'de', 'ja', 'sw']);
    expect(languages[0]).toEqual({ code: 'en', name: 'English' });
    expect(modelsFor([unified, nemotron, whisper], 'ja').map((entry) => entry.id)).toEqual(['nemotron', 'whisper']);
    expect(modelsFor([unified, nemotron, whisper], 'all')).toHaveLength(3);
  });
});
