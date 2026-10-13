import { describe, expect, it } from 'vitest';

import { pickModel, spokenLanguage, type PickableModel } from './transcription-pick';

const unified: PickableModel = { id: 'unified', languages: 'en', languageCodes: ['en'], wordTimestamps: true, detectsLanguage: false };
const v2: PickableModel = { id: 'v2', languages: 'en', languageCodes: ['en'], wordTimestamps: true, detectsLanguage: false };
const nemotron: PickableModel = { id: 'nemotron', languages: 'multi', languageCodes: ['en', 'de', 'ko'], wordTimestamps: true, detectsLanguage: true };
const v3: PickableModel = { id: 'v3', languages: 'multi', languageCodes: ['en', 'de', 'fr'], wordTimestamps: true, detectsLanguage: true };
const whisper: PickableModel = { id: 'whisper', languages: 'multi', languageCodes: ['en', 'de', 'fr', 'ko'], wordTimestamps: false, detectsLanguage: true };
const cohere: PickableModel = { id: 'cohere', languages: 'multi', languageCodes: ['en', 'de', 'ko'], wordTimestamps: false, detectsLanguage: false };

const captions = (language = 'auto') => ({ wordTimestamps: true, language });

describe('spokenLanguage', () => {
  it('is the chosen tag, English for a model that cannot detect, and unknown otherwise', () => {
    expect(spokenLanguage(whisper, 'de-AT')).toBe('de');
    expect(spokenLanguage(whisper, 'auto')).toBe('');
    expect(spokenLanguage(cohere, 'auto')).toBe('en');
    expect(spokenLanguage(unified, undefined)).toBe('en');
    expect(spokenLanguage(null, 'auto')).toBe('');
  });
});

describe('pickModel', () => {
  it('keeps the active model unless captions need word timing it lacks', () => {
    expect(pickModel(whisper, [whisper, unified])).toBe(whisper);
    expect(pickModel(nemotron, [nemotron, unified], captions())).toBe(nemotron);
    expect(pickModel(null, [])).toBeNull();
  });

  it('never sends a multilingual model’s unknown speech to an English-only stand-in', () => {
    // Whisper on automatic for Korean interviews, Unified EN from onboarding: no silent English captions.
    expect(pickModel(whisper, [unified, whisper], captions())).toBeNull();
    expect(pickModel(whisper, [unified, v2, whisper], captions())).toBeNull();
    // A downloaded multilingual model that detects the language stands in, even listed after Unified EN.
    expect(pickModel(whisper, [unified, nemotron, whisper], captions())).toBe(nemotron);
  });

  it('matches a known spoken language, and finds nothing when no word-timed model hears it', () => {
    expect(pickModel(whisper, [unified, nemotron, whisper], captions('en-US'))).toBe(unified);
    expect(pickModel(whisper, [unified, v3, nemotron, whisper], captions('ko'))).toBe(nemotron);
    expect(pickModel(whisper, [unified, v3, whisper], captions('ko'))).toBeNull();
  });

  it('hears English for a model that cannot detect the language', () => {
    expect(pickModel(cohere, [unified, cohere], captions())).toBe(unified);
    expect(pickModel(cohere, [unified, nemotron, cohere], captions('de'))).toBe(nemotron);
  });
});
