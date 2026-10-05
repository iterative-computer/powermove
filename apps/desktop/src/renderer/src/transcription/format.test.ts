import { describe, expect, it } from 'vitest';

import type { TranscriptionModelInfo, TranscriptionStatus } from '../../../shared/transcription';
import { catalogLanguages, curatedModels, modelFacts, modelFor, modelsFor, reach } from './format';

const model = (id: string, extra: Partial<TranscriptionModelInfo> = {}): TranscriptionModelInfo => ({
  id, name: id, description: '', size: 731_357_568, languages: 'multi', languageCodes: ['en', 'de', 'fr'],
  speed: 0.8, accuracy: 0.8, recommended: false, wordTimestamps: true, state: 'absent', ...extra
});

const unified = model('unified', { languages: 'en', languageCodes: ['en'], recommended: true, featured: true });
const nemotron = model('nemotron', { languageCodes: ['en', 'de', 'ja'], recommended: true, featured: true });
const whisper = model('whisper', { wordTimestamps: false, timing: 'segment', languageCodes: ['en', 'de', 'fr', 'ja', 'sw'] });
const cohere = model('cohere', { wordTimestamps: false, timing: 'none', size: 1_770_270_208 });

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
    expect(modelFor(status([ready(unified), ready(nemotron), ready(whisper)], 'whisper'), { wordTimestamps: true })?.id).toBe('unified');
    // Downloading is not ready.
    expect(modelFor(status([{ ...unified, state: 'downloading' }, ready(whisper)], 'whisper'), { wordTimestamps: true })).toBeNull();
    expect(modelFor(status([], null))).toBeNull();
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
