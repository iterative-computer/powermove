import { describe, expect, it } from 'vitest';

import { CATALOG, DEFAULT_MODEL_ID, decodeLanguage, modelFile, modelSize, parseCatalog } from './catalog';

describe('the pinned catalog', () => {
  it('pins every model to one GGUF at a Hugging Face revision with its checksum', () => {
    for (const model of CATALOG) {
      expect(model.files).toHaveLength(1);
      const file = modelFile(model);
      expect(file.name).toMatch(/\.gguf$/);
      expect(file.url).toBe(`${file.url.split('/resolve/')[0]}/resolve/${file.url.split('/resolve/')[1]!.split('/')[0]}/${file.name}`);
      expect(file.url).toMatch(/^https:\/\/huggingface\.co\/handy-computer\/[\w.-]+-gguf\/resolve\/[0-9a-f]{40}\//);
      expect(file.sha256).toMatch(/^[0-9a-f]{64}$/);
      expect(modelSize(model)).toBeGreaterThan(100e6);
    }
    expect(new Set(CATALOG.map((model) => model.id)).size).toBe(CATALOG.length);
    // A pinned catalog survives its own validation unchanged (the e2e override path).
    expect(parseCatalog(JSON.parse(JSON.stringify(CATALOG)))).toEqual(CATALOG);
  });

  it('offers Handy\'s out-of-box set', () => {
    expect(CATALOG.map((model) => model.id)).toEqual(expect.arrayContaining([
      'parakeet-unified-en-0.6b', 'nemotron-3.5-asr-streaming-0.6b', 'canary-180m-flash',
      'cohere-transcribe-03-2026', 'whisper-medium', 'parakeet-tdt-0.6b-v3', 'parakeet-tdt-0.6b-v2'
    ]));
  });

  it('recommends one English and one multilingual model, and curates at most three', () => {
    const recommended = CATALOG.filter((model) => model.recommended);
    expect(recommended.map((model) => model.languages).sort()).toEqual(['en', 'multi']);
    expect(recommended.find((model) => model.languages === 'en')!.id).toBe(DEFAULT_MODEL_ID);
    expect(recommended.every((model) => model.wordTimestamps && model.featured)).toBe(true);
    const featured = CATALOG.filter((model) => model.featured);
    expect(featured.length).toBeLessThanOrEqual(3);
    // Onboarding and the sheet promise captions: every curated model times words.
    expect(featured.every((model) => model.wordTimestamps)).toBe(true);
  });

  it('claims word timestamps only for token-timed models', () => {
    for (const model of CATALOG) {
      expect(model.wordTimestamps).toBe(model.timing === 'word');
      if (model.lag) expect(model.timing).toBe('word');
    }
    expect(CATALOG.filter((model) => !model.wordTimestamps).map((model) => model.id).sort())
      .toEqual(['canary-180m-flash', 'cohere-transcribe-03-2026', 'whisper-medium']);
  });

  it('lists languages consistently with the English-only flag', () => {
    for (const model of CATALOG) {
      if (model.languages === 'en') expect(model.languageCodes).toEqual(['en']);
      else expect(model.languageCodes.length).toBeGreaterThan(1);
      expect(model.languageCodes.every((code) => /^[a-z]{2,3}$/.test(code))).toBe(true);
    }
    expect(CATALOG.find((model) => model.id === 'whisper-medium')!.languageCodes).toHaveLength(99);
    expect(CATALOG.find((model) => model.id === 'nemotron-3.5-asr-streaming-0.6b')!.languageCodes).toHaveLength(28);
  });
});

describe('decodeLanguage', () => {
  const detecting = { languages: 'multi' as const, languageCodes: ['en', 'de', 'fr'], detectsLanguage: true };
  const told = { languages: 'multi' as const, languageCodes: ['en', 'de', 'es', 'fr'], detectsLanguage: false };
  const english = { languages: 'en' as const, languageCodes: ['en'], detectsLanguage: false };

  it('hands a model a language it lists, lets detectors detect, and tells the others English', () => {
    expect(decodeLanguage(detecting, 'de-AT')).toBe('de');
    expect(decodeLanguage(detecting, 'auto')).toBeUndefined();
    expect(decodeLanguage(detecting, 'ja')).toBeUndefined();
    expect(decodeLanguage(told, 'fr')).toBe('fr');
    expect(decodeLanguage(told, 'auto')).toBe('en');
    expect(decodeLanguage(told, 'ja')).toBe('en');
    expect(decodeLanguage({ ...told, languageCodes: ['ar'] }, undefined)).toBe('ar');
    expect(decodeLanguage(english, 'de')).toBeUndefined();
  });
});

describe('parseCatalog', () => {
  const file = { name: 'a.gguf', url: 'http://127.0.0.1/a.gguf', size: 10, sha256: 'a'.repeat(64) };

  it('fills defaults and derives word timing from the timing kind', () => {
    const [model] = parseCatalog([{ id: 'a', files: [file], timing: 'segment', languages: 'multi', languageCodes: ['en', 'de', 'English'], lag: [0.2, 0.1] }]);
    expect(model).toMatchObject({ id: 'a', timing: 'segment', wordTimestamps: false, detectsLanguage: true, languageCodes: ['en', 'de'], featured: false, family: 'parakeet' });
    // A lag on a model without word timing is kept; the worker ignores it.
    expect(model!.lag).toEqual([0.2, 0.1]);
    expect(parseCatalog([{ id: 'b', files: [file] }])[0]).toMatchObject({ timing: 'word', wordTimestamps: true });
    expect(parseCatalog([{ id: 'c', files: [file], languages: 'en', languageCodes: ['de'] }])[0]!.languageCodes).toEqual(['en']);
  });

  it('rejects anything off', () => {
    expect(() => parseCatalog([])).toThrow();
    expect(() => parseCatalog([{ id: '../x', files: [file] }])).toThrow();
    expect(() => parseCatalog([{ id: 'a', files: [file, file] }])).toThrow(/exactly one/);
    expect(() => parseCatalog([{ id: 'a', files: [{ ...file, sha256: 'nope' }] }])).toThrow();
    expect(() => parseCatalog([{ id: 'a', files: [{ ...file, name: '../a.gguf' }] }])).toThrow();
    expect(parseCatalog([{ id: 'a', files: [file], lag: [5, 0] }])[0]!.lag).toBeUndefined();
  });
});
