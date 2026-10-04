/*
 * The downloadable speech models. Nothing here ships in the app: each model
 * is a handful of ONNX files fetched on request into the user data folder.
 *
 * Same picks as Handy's ONNX catalog (cjpais/Handy, managers/model.rs), whose
 * default is Parakeet V3: NVIDIA Parakeet TDT 0.6B, int8, run by sherpa-onnx.
 * The files are k2-fsa's sherpa-onnx exports on Hugging Face, pinned to a
 * repository revision and verified by SHA-256 after download. The LFS
 * checksums are Hugging Face's own; tokens.txt is not in LFS and was hashed
 * from the pinned revision. Bump the revision and every checksum together.
 */

export type TranscriptionModelKind = 'nemo-transducer';

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
  kind: TranscriptionModelKind;
  languages: 'en' | 'multi';
  languageCodes: string[];
  speed: number;
  accuracy: number;
  recommended: boolean;
  wordTimestamps: boolean;
  license: string;
  files: CatalogFile[];
}

const HF = 'https://huggingface.co';

function files(repo: string, revision: string, entries: Array<[name: string, size: number, sha256: string]>): CatalogFile[] {
  return entries.map(([name, size, sha256]) => ({ name, size, sha256, url: `${HF}/${repo}/resolve/${revision}/${name}` }));
}

/** Parakeet TDT 0.6B v3's 25 European languages (NVIDIA model card). */
export const PARAKEET_V3_LANGUAGES = [
  'bg', 'hr', 'cs', 'da', 'nl', 'en', 'et', 'fi', 'fr', 'de', 'el', 'hu', 'it',
  'lv', 'lt', 'mt', 'pl', 'pt', 'ro', 'sk', 'sl', 'es', 'sv', 'ru', 'uk'
];

export const CATALOG: readonly CatalogModel[] = [
  {
    id: 'parakeet-tdt-0.6b-v3',
    name: 'Parakeet V3',
    description: 'Fast and accurate in 25 European languages.',
    kind: 'nemo-transducer',
    languages: 'multi',
    languageCodes: PARAKEET_V3_LANGUAGES,
    speed: 0.85,
    accuracy: 0.8,
    recommended: true,
    wordTimestamps: true,
    license: 'CC BY 4.0 · NVIDIA',
    files: files('csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8', '2bda32ec70b097a55adaa07d9a7173915b43cc78', [
      ['encoder.int8.onnx', 652_184_281, 'acfc2b4456377e15d04f0243af540b7fe7c992f8d898d751cf134c3a55fd2247'],
      ['decoder.int8.onnx', 11_845_275, '179e50c43d1a9de79c8a24149a2f9bac6eb5981823f2a2ed88d655b24248db4e'],
      ['joiner.int8.onnx', 6_355_277, '3164c13fc2821009440d20fcb5fdc78bff28b4db2f8d0f0b329101719c0948b3'],
      ['tokens.txt', 93_939, 'd58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d']
    ])
  },
  {
    id: 'parakeet-tdt-0.6b-v2',
    name: 'Parakeet V2',
    description: 'English only. The most accurate for English.',
    kind: 'nemo-transducer',
    languages: 'en',
    languageCodes: ['en'],
    speed: 0.85,
    accuracy: 0.85,
    recommended: false,
    wordTimestamps: true,
    license: 'CC BY 4.0 · NVIDIA',
    files: files('csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8', '1ab9323565ddb038682214b292f588070a538ce2', [
      ['encoder.int8.onnx', 652_184_296, 'a32b12d17bbbc309d0686fbbcc2987b5e9b8333a7da83fa6b089f0a2acd651ab'],
      ['decoder.int8.onnx', 7_257_753, 'b6bb64963457237b900e496ee9994b59294526439fbcc1fecf705b31a15c6b4e'],
      ['joiner.int8.onnx', 1_739_080, '7946164367946e7f9f29a122407c3252b680dbae9a51343eb2488d057c3c43d2'],
      ['tokens.txt', 9_384, 'ec182b70dd42113aff6c5372c75cac58c952443eb22322f57bbd7f53977d497d']
    ])
  }
];

export const DEFAULT_MODEL_ID = 'parakeet-tdt-0.6b-v3';

export function modelSize(model: CatalogModel): number {
  return model.files.reduce((sum, file) => sum + file.size, 0);
}

const ID = /^[a-z0-9][a-z0-9._-]{0,63}$/;
const NAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;
const SHA = /^[0-9a-f]{64}$/;

/** Validates a catalog read from disk (the e2e override). Throws on anything off. */
export function parseCatalog(value: unknown): CatalogModel[] {
  if (!Array.isArray(value) || value.length === 0 || value.length > 16) throw new Error('A catalog is a non-empty list of models.');
  return value.map((entry) => {
    const model = entry as Partial<CatalogModel>;
    if (!model || typeof model !== 'object' || typeof model.id !== 'string' || !ID.test(model.id)) throw new Error('Catalog model id is invalid.');
    if (!Array.isArray(model.files) || !model.files.length) throw new Error(`Catalog model ${model.id} has no files.`);
    for (const file of model.files) {
      if (!file || typeof file.name !== 'string' || !NAME.test(file.name) || typeof file.url !== 'string' || !/^https?:\/\//.test(file.url)
        || !Number.isSafeInteger(file.size) || file.size <= 0 || typeof file.sha256 !== 'string' || !SHA.test(file.sha256)) {
        throw new Error(`Catalog model ${model.id} has an invalid file.`);
      }
    }
    return {
      id: model.id,
      name: String(model.name ?? model.id),
      description: String(model.description ?? ''),
      kind: 'nemo-transducer',
      languages: model.languages === 'en' ? 'en' : 'multi',
      languageCodes: Array.isArray(model.languageCodes) ? model.languageCodes.filter((code): code is string => typeof code === 'string') : [],
      speed: Number(model.speed) || 0,
      accuracy: Number(model.accuracy) || 0,
      recommended: model.recommended === true,
      wordTimestamps: model.wordTimestamps !== false,
      license: String(model.license ?? ''),
      files: model.files.map((file) => ({ name: file.name, url: file.url, size: file.size, sha256: file.sha256 }))
    };
  });
}
