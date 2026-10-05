/*
 * The downloadable speech models. Nothing here ships in the app: each model
 * is one GGUF file fetched on request into the user data folder and run by
 * transcribe.cpp (ggml, Metal on Apple Silicon) in the transcription process.
 *
 * Same out-of-box set as Handy (cjpais/Handy, src-tauri/src/catalog/
 * catalog.json, catalog_version 2): its five recommended models plus
 * Parakeet V3 and V2, with Handy's default quantisation, speed and accuracy
 * scores. The files are the handy-computer GGUF conversions on Hugging Face,
 * each numerically verified against its reference model by transcribe.cpp.
 * Every file is pinned to a repository revision and checked against the
 * SHA-256 the catalog publishes (the same as Hugging Face's LFS checksum).
 * Bump a revision and its checksum and size together.
 *
 * Not offered, with reasons: Voxtral Mini 4B Realtime (3.3 GB, slowest in
 * Handy's table and no timing at all, too heavy for long media), Qwen3-ASR
 * and Fun-ASR-Nano (LLM decoders without timing that duplicate what Cohere
 * and Canary already cover), and the rest of Handy's long tail (Moonshine,
 * GigaAM, Granite, … single-language or experimental).
 */

/** How finely a model times its output: 'word' from token timestamps,
 *  'segment' from phrase timestamps (Whisper), 'none' for text only. */
export type CatalogTiming = 'word' | 'segment' | 'none';

export type CatalogFamily = 'parakeet' | 'canary' | 'cohere' | 'whisper';

export interface CatalogFile {
  /** File name inside the model folder. */
  name: string;
  url: string;
  size: number;
  sha256: string;
}

export interface CatalogModel {
  id: string;
  name: string;
  description: string;
  family: CatalogFamily;
  languages: 'en' | 'multi';
  languageCodes: string[];
  /** False when the model must be told the language (Canary, Cohere). */
  detectsLanguage: boolean;
  timing: CatalogTiming;
  speed: number;
  accuracy: number;
  /** The default for its kind: one English, one multilingual. */
  recommended: boolean;
  /** Shown in the download sheet and onboarding (at most three, all
   *  word-timed so whatever is picked there can make captions). The rest
   *  are in Settings › Transcription. */
  featured: boolean;
  /** Derived from timing: only 'word' models time each word. */
  wordTimestamps: boolean;
  /**
   * Seconds this model reports words late, as [start, end]. Streaming RNNT
   * models (Unified EN, Nemotron) emit a token a little after they hear it;
   * measured against Parakeet TDT V2 and the audio's own onsets on say(1)
   * speech (feat/transcription-catalog report), then taken off every word.
   */
  lag?: [start: number, end: number];
  license: string;
  /** Exactly one file: the GGUF the worker loads. */
  files: CatalogFile[];
}

const HF = 'https://huggingface.co';

function gguf(repo: string, revision: string, name: string, size: number, sha256: string): CatalogFile[] {
  return [{ name, size, sha256, url: `${HF}/${repo}/resolve/${revision}/${name}` }];
}

/** Parakeet TDT 0.6B v3's 25 European languages (NVIDIA model card). */
export const PARAKEET_V3_LANGUAGES = [
  'bg', 'hr', 'cs', 'da', 'nl', 'en', 'et', 'fi', 'fr', 'de', 'el', 'hu', 'it',
  'lv', 'lt', 'mt', 'pl', 'pt', 'ro', 'sk', 'sl', 'es', 'sv', 'ru', 'uk'
];

/** Nemotron 3.5 ASR Streaming's 28 languages (the model reports regional tags such as de-DE). */
export const NEMOTRON_LANGUAGES = [
  'en', 'es', 'fr', 'it', 'pt', 'nl', 'de', 'tr', 'ru', 'ar', 'hi', 'ja', 'ko', 'vi',
  'uk', 'pl', 'sv', 'cs', 'nb', 'da', 'bg', 'fi', 'hr', 'sk', 'zh', 'hu', 'ro', 'et'
];

export const COHERE_LANGUAGES = ['en', 'fr', 'de', 'es', 'it', 'pt', 'nl', 'pl', 'el', 'ar', 'ja', 'zh', 'vi', 'ko'];

/** Whisper's 99 languages, in Whisper's own order. */
export const WHISPER_LANGUAGES = [
  'en', 'zh', 'de', 'es', 'ru', 'ko', 'fr', 'ja', 'pt', 'tr', 'pl', 'ca', 'nl', 'ar', 'sv', 'it', 'id', 'hi',
  'fi', 'vi', 'he', 'uk', 'el', 'ms', 'cs', 'ro', 'da', 'hu', 'ta', 'no', 'th', 'ur', 'hr', 'bg', 'lt', 'la',
  'mi', 'ml', 'cy', 'sk', 'te', 'fa', 'lv', 'bn', 'sr', 'az', 'sl', 'kn', 'et', 'mk', 'br', 'eu', 'is', 'hy',
  'ne', 'mn', 'bs', 'kk', 'sq', 'sw', 'gl', 'mr', 'pa', 'si', 'km', 'sn', 'yo', 'so', 'af', 'oc', 'ka', 'be',
  'tg', 'sd', 'gu', 'am', 'yi', 'lo', 'uz', 'fo', 'ht', 'ps', 'tk', 'nn', 'mt', 'sa', 'lb', 'my', 'bo', 'tl',
  'mg', 'as', 'tt', 'haw', 'ln', 'ha', 'ba', 'jw', 'su'
];

const NVIDIA = 'CC BY 4.0 · NVIDIA';

export const CATALOG: readonly CatalogModel[] = [
  {
    id: 'parakeet-unified-en-0.6b',
    name: 'Parakeet Unified EN',
    description: 'Fast, accurate English.',
    family: 'parakeet',
    languages: 'en',
    languageCodes: ['en'],
    detectsLanguage: false,
    timing: 'word',
    speed: 0.79,
    accuracy: 0.9,
    recommended: true,
    featured: true,
    wordTimestamps: true,
    lag: [0.32, 0.08],
    license: NVIDIA,
    files: gguf('handy-computer/parakeet-unified-en-0.6b-gguf', '7e948f21b7bdbac698d3318db9d350f1096f3b6c',
      'parakeet-unified-en-0.6b-Q8_0.gguf', 731_357_568, '4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795')
  },
  {
    id: 'nemotron-3.5-asr-streaming-0.6b',
    name: 'Nemotron Streaming 3.5',
    description: 'Fast multilingual transcription across 28 languages.',
    family: 'parakeet',
    languages: 'multi',
    languageCodes: NEMOTRON_LANGUAGES,
    detectsLanguage: true,
    timing: 'word',
    speed: 0.84,
    accuracy: 0.82,
    recommended: true,
    featured: true,
    wordTimestamps: true,
    lag: [0.3, 0.08],
    license: 'OpenMDW 1.1 · NVIDIA',
    files: gguf('handy-computer/nemotron-3.5-asr-streaming-0.6b-gguf', '6d44e540bc31b0de1dbe174a3cea87f53a7f22fb',
      'nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf', 751_094_240, 'b94545b313b3223fda7b2857a52681da813935c2127643d1e9ff0c23d988089c')
  },
  {
    id: 'parakeet-tdt-0.6b-v3',
    name: 'Parakeet V3',
    description: 'Fast and accurate in 25 European languages.',
    family: 'parakeet',
    languages: 'multi',
    languageCodes: PARAKEET_V3_LANGUAGES,
    detectsLanguage: true,
    timing: 'word',
    speed: 0.79,
    accuracy: 0.88,
    recommended: false,
    featured: true,
    wordTimestamps: true,
    license: NVIDIA,
    files: gguf('handy-computer/parakeet-tdt-0.6b-v3-gguf', '85ac09ea12fc4b1112fa76810059364bc6adc9de',
      'parakeet-tdt-0.6b-v3-Q8_0.gguf', 739_508_576, '5859f77944efcd8eafa23a6350731960b2b55b2203df51f319665c807d802cc7')
  },
  {
    id: 'parakeet-tdt-0.6b-v2',
    name: 'Parakeet V2',
    description: 'English only. A little faster than Unified EN.',
    family: 'parakeet',
    languages: 'en',
    languageCodes: ['en'],
    detectsLanguage: false,
    timing: 'word',
    speed: 0.85,
    accuracy: 0.89,
    recommended: false,
    featured: false,
    wordTimestamps: true,
    license: NVIDIA,
    files: gguf('handy-computer/parakeet-tdt-0.6b-v2-gguf', '07cee0616125a08ef619729bb47f40ef747e4bc4',
      'parakeet-tdt-0.6b-v2-Q8_0.gguf', 729_574_912, 'f0d0e99cebb6d3b83f1f7069b82b5d3c2e39a54545b0da039cb4bafd9c4e5caa')
  },
  {
    id: 'whisper-medium',
    name: 'Whisper Medium',
    description: 'The broadest language coverage, a little slower.',
    family: 'whisper',
    languages: 'multi',
    languageCodes: WHISPER_LANGUAGES,
    detectsLanguage: true,
    timing: 'segment',
    speed: 0.42,
    accuracy: 0.84,
    recommended: false,
    featured: false,
    wordTimestamps: false,
    license: 'Apache 2.0 · OpenAI',
    files: gguf('handy-computer/whisper-medium-gguf', 'ec78f06fded51aa82cde751678b78f76f78c8b7f',
      'whisper-medium-Q8_0.gguf', 831_538_144, '09e6a65e7de377aa5b10bae24608bc6f8ca2ed04b3993ef10d4a02bcd9a82adf')
  },
  {
    id: 'canary-180m-flash',
    name: 'Canary 180M Flash',
    description: 'Tiny and instant in English, German, Spanish and French.',
    family: 'canary',
    languages: 'multi',
    languageCodes: ['en', 'de', 'es', 'fr'],
    detectsLanguage: false,
    timing: 'none',
    speed: 0.98,
    accuracy: 0.88,
    recommended: false,
    featured: false,
    wordTimestamps: false,
    license: NVIDIA,
    files: gguf('handy-computer/canary-180m-flash-gguf', 'b147f9dc52b59f0998e410540a84727bd86457fd',
      'canary-180m-flash-Q8_0.gguf', 218_447_552, 'e13c7f5d0952b056a027cfffec13e3a3a134d1608babed24f983568f141e297c')
  },
  {
    id: 'cohere-transcribe-03-2026',
    name: 'Cohere Transcribe',
    description: 'The most accurate in 14 languages, and slower.',
    family: 'cohere',
    languages: 'multi',
    languageCodes: COHERE_LANGUAGES,
    detectsLanguage: false,
    timing: 'none',
    speed: 0.63,
    accuracy: 0.92,
    recommended: false,
    featured: false,
    wordTimestamps: false,
    license: 'Apache 2.0 · Cohere',
    files: gguf('handy-computer/cohere-transcribe-03-2026-gguf', 'dfa4adebb64f3076b7b6b90b721275cc069cb421',
      'cohere-transcribe-03-2026-Q5_K_M.gguf', 1_770_270_208, '14d02f1ad6dd77b3a60f82639879012c3adb4fe25c50a5a47a2c4c661daf1558')
  }
];

export const DEFAULT_MODEL_ID = 'parakeet-unified-en-0.6b';

export function modelSize(model: CatalogModel): number {
  return model.files.reduce((sum, file) => sum + file.size, 0);
}

/** The GGUF the worker loads (a catalog model has exactly one file). */
export function modelFile(model: CatalogModel): CatalogFile {
  return model.files[0]!;
}

/**
 * The tag to hand the model for a requested language, or undefined to let
 * it detect (or, for an English-only model, to say nothing). Models that
 * cannot detect fall back to English, else their first language.
 */
export function decodeLanguage(model: Pick<CatalogModel, 'languages' | 'languageCodes' | 'detectsLanguage'>, requested: string | undefined): string | undefined {
  if (model.languages === 'en') return undefined;
  const base = requested && requested !== 'auto' ? requested.split('-')[0]!.toLowerCase() : '';
  if (base && model.languageCodes.includes(base)) return base;
  if (model.detectsLanguage) return undefined;
  return model.languageCodes.includes('en') ? 'en' : model.languageCodes[0];
}

const ID = /^[a-z0-9][a-z0-9._-]{0,63}$/;
const NAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;
const SHA = /^[0-9a-f]{64}$/;
const TAG = /^[a-z]{2,3}$/;
const FAMILIES: readonly CatalogFamily[] = ['parakeet', 'canary', 'cohere', 'whisper'];
const TIMINGS: readonly CatalogTiming[] = ['word', 'segment', 'none'];

function validLag(value: unknown): value is [number, number] {
  return Array.isArray(value) && value.length === 2 && value.every((entry) => typeof entry === 'number' && Number.isFinite(entry) && Math.abs(entry) <= 1);
}

/** Validates a catalog read from disk (the e2e override). Throws on anything off. */
export function parseCatalog(value: unknown): CatalogModel[] {
  if (!Array.isArray(value) || value.length === 0 || value.length > 16) throw new Error('A catalog is a non-empty list of models.');
  return value.map((entry) => {
    const model = entry as Partial<CatalogModel>;
    if (!model || typeof model !== 'object' || typeof model.id !== 'string' || !ID.test(model.id)) throw new Error('Catalog model id is invalid.');
    if (!Array.isArray(model.files) || model.files.length !== 1) throw new Error(`Catalog model ${model.id} needs exactly one file.`);
    for (const file of model.files) {
      if (!file || typeof file.name !== 'string' || !NAME.test(file.name) || typeof file.url !== 'string' || !/^https?:\/\//.test(file.url)
        || !Number.isSafeInteger(file.size) || file.size <= 0 || typeof file.sha256 !== 'string' || !SHA.test(file.sha256)) {
        throw new Error(`Catalog model ${model.id} has an invalid file.`);
      }
    }
    const timing: CatalogTiming = TIMINGS.includes(model.timing as CatalogTiming) ? model.timing as CatalogTiming : 'word';
    const languages = model.languages === 'en' ? 'en' : 'multi';
    const codes = Array.isArray(model.languageCodes) ? model.languageCodes.filter((code): code is string => typeof code === 'string' && TAG.test(code)) : [];
    return {
      id: model.id,
      name: String(model.name ?? model.id),
      description: String(model.description ?? ''),
      family: FAMILIES.includes(model.family as CatalogFamily) ? model.family as CatalogFamily : 'parakeet',
      languages,
      languageCodes: languages === 'en' ? ['en'] : codes,
      detectsLanguage: languages === 'multi' && model.detectsLanguage !== false,
      timing,
      speed: Number(model.speed) || 0,
      accuracy: Number(model.accuracy) || 0,
      recommended: model.recommended === true,
      featured: model.featured === true,
      wordTimestamps: timing === 'word',
      ...(validLag(model.lag) ? { lag: [model.lag[0], model.lag[1]] as [number, number] } : {}),
      license: String(model.license ?? ''),
      files: model.files.map((file) => ({ name: file.name, url: file.url, size: file.size, sha256: file.sha256 }))
    };
  });
}
