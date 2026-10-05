// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';

import { TRANSCRIPTION_MODEL_MISSING, type TranscriptionBridge, type TranscriptionStatus } from '../../../shared/transcription';
import { installBridgeForTests } from '../kernel/bridge';
import { ensureTranscriptionModel, transcribe, transcriptionStatus } from './client';
import { downloadNotice } from './install';
import { formatBytes, languageOptions, percent } from './format';

const model = (state: 'absent' | 'downloading' | 'ready' | 'error', extra = {}) => ({
  id: 'parakeet-tdt-0.6b-v3', name: 'Parakeet V3', description: 'Fast.', size: 670_478_772, languages: 'multi' as const,
  languageCodes: ['en', 'de'], speed: 0.85, accuracy: 0.8, recommended: true, wordTimestamps: true, state, ...extra
});

function host(status: TranscriptionStatus, overrides: Partial<TranscriptionBridge> = {}): TranscriptionBridge {
  return {
    status: vi.fn(async () => status),
    download: vi.fn(async () => status),
    cancelDownload: vi.fn(async () => status),
    remove: vi.fn(async () => status),
    setActive: vi.fn(async () => status),
    setLanguage: vi.fn(async () => status),
    reveal: vi.fn(async () => undefined),
    transcribe: vi.fn(async () => ({ ok: true as const, transcript: { modelId: 'm', duration: 1, segments: [] } })),
    cancelTranscribe: vi.fn(async () => undefined),
    onStatus: vi.fn(() => () => undefined),
    onProgress: vi.fn(() => () => undefined),
    onModelRequested: vi.fn(() => () => undefined),
    ...overrides
  };
}

function install(transcription: TranscriptionBridge | undefined) {
  installBridgeForTests({ transcription } as never);
}

afterEach(() => installBridgeForTests(undefined));

describe('transcription client', () => {
  it('reports unavailable without a host bridge, and keeps the frozen missing-model code', async () => {
    install(undefined);
    expect(await transcriptionStatus()).toMatchObject({ available: false, activeModelId: null });
    expect(await ensureTranscriptionModel('why')).toBe(false);
    await expect(transcribe({ path: '/a.wav' })).rejects.toMatchObject({ code: TRANSCRIPTION_MODEL_MISSING });
  });

  it('resolves true at once when a model is in use, false on an unavailable host', async () => {
    install(host({ models: [model('ready')], activeModelId: 'parakeet-tdt-0.6b-v3' }));
    expect(await ensureTranscriptionModel('why')).toBe(true);
    install(host({ models: [], activeModelId: null, available: false }));
    expect(await ensureTranscriptionModel('why')).toBe(false);
  });

  it('asks for a word-timed model only when captions need one and none is downloaded', async () => {
    const whisper = model('ready', { id: 'whisper-medium', name: 'Whisper Medium', wordTimestamps: false, timing: 'segment' });
    install(host({ models: [whisper], activeModelId: 'whisper-medium' }));
    expect(await ensureTranscriptionModel('why')).toBe(true);
    // No sheet host in this test: the sheet cannot open, so the answer is no.
    expect(await ensureTranscriptionModel('why', { wordTimestamps: true })).toBe(false);
    install(host({ models: [whisper, model('ready')], activeModelId: 'whisper-medium' }));
    expect(await ensureTranscriptionModel('why', { wordTimestamps: true })).toBe(true);
  });

  it('turns an error envelope into an error with its code', async () => {
    install(host({ models: [], activeModelId: null }, {
      transcribe: vi.fn(async () => ({ ok: false as const, code: TRANSCRIPTION_MODEL_MISSING, message: 'No transcription model is downloaded yet.' }))
    }));
    await expect(transcribe({ path: '/a.wav' })).rejects.toMatchObject({ code: TRANSCRIPTION_MODEL_MISSING, message: 'No transcription model is downloaded yet.' });
  });

  it('streams progress for its own request and cancels through the signal', async () => {
    let progressListener: ((event: { requestId: string; progress: number }) => void) | null = null;
    let finish!: (value: unknown) => void;
    const bridge = host({ models: [], activeModelId: 'm' }, {
      onProgress: vi.fn((callback) => { progressListener = callback; return () => { progressListener = null; }; }),
      transcribe: vi.fn(() => new Promise((resolve) => { finish = resolve; })) as never
    });
    install(bridge);
    const seen: number[] = [];
    const controller = new AbortController();
    const pending = transcribe({ path: '/a.wav' }, (value) => seen.push(value), controller.signal);
    await Promise.resolve();
    const requestId = (bridge.transcribe as ReturnType<typeof vi.fn>).mock.calls[0]![0].requestId as string;
    progressListener!({ requestId: 'someone-else', progress: 0.9 });
    progressListener!({ requestId, progress: 0.4 });
    controller.abort();
    expect(bridge.cancelTranscribe).toHaveBeenCalledWith(requestId);
    finish({ ok: false, code: 'aborted', message: 'Transcription was cancelled.' });
    await expect(pending).rejects.toMatchObject({ name: 'AbortError', code: 'aborted' });
    expect(seen).toEqual([0.4]);
    expect(progressListener).toBeNull();
  });
});

describe('download notice', () => {
  it('follows a download to ready or failure', () => {
    const absent = { models: [model('absent')], activeModelId: null };
    const downloading = { models: [model('downloading', { progress: 0.25 })], activeModelId: null };
    expect(downloadNotice(absent, downloading)).toEqual({ message: 'Downloading Parakeet V3', progress: 0.25 });
    expect(downloadNotice(downloading, { models: [model('ready')], activeModelId: 'parakeet-tdt-0.6b-v3' }))
      .toEqual({ message: 'Parakeet V3 is ready for transcription', progress: 1, done: true });
    expect(downloadNotice(downloading, { models: [model('error', { error: 'The download stalled.' })], activeModelId: null }))
      .toMatchObject({ error: true, message: 'Parakeet V3 could not be downloaded. The download stalled.' });
    expect(downloadNotice(absent, absent)).toBeNull();
  });
});

describe('format', () => {
  it('names sizes and languages', () => {
    expect(formatBytes(670_478_772)).toBe('670 MB');
    expect(formatBytes(1_340_000_000)).toBe('1.3 GB');
    expect(percent(0.456)).toBe('45%');
    expect(languageOptions(model('ready')).map((option) => option.name)).toEqual(['English', 'German']);
  });
});
