import { mkdtempSync, readFileSync, rmSync, existsSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { IPC } from '../shared/ipc';
import { registerLogIpc } from './log';

describe('renderer log', () => {
  const dirs: string[] = [];
  afterEach(() => {
    delete process.env.POWERMOVE_NATIVE_REPORT_FILE;
    vi.restoreAllMocks();
    for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
  });

  const listen = () => {
    let listener: ((event: any, payload: unknown) => void) | undefined;
    registerLogIpc({ on: (channel: string, fn: any) => { if (channel === IPC.log) listener = fn; } } as any, { isTrustedSenderContents: () => true });
    return (text: string) => listener!({ sender: {} }, { level: 'warn', text });
  };

  it('appends native evaluator reports to the report file a test run names', () => {
    vi.spyOn(process.stderr, 'write').mockImplementation(() => true);
    const dir = mkdtempSync(path.join(os.tmpdir(), 'pm-log-'));
    dirs.push(dir);
    const file = path.join(dir, 'report.jsonl');
    process.env.POWERMOVE_NATIVE_REPORT_FILE = file;
    const send = listen();
    send('[pm-native] {"type":"ready"}');
    send('something else');
    send('[pm-native] {"type":"summary"}');
    expect(readFileSync(file, 'utf8')).toBe('{"type":"ready"}\n{"type":"summary"}\n');
  });

  it('writes no report file unless asked', () => {
    vi.spyOn(process.stderr, 'write').mockImplementation(() => true);
    const dir = mkdtempSync(path.join(os.tmpdir(), 'pm-log-'));
    dirs.push(dir);
    listen()('[pm-native] {"type":"ready"}');
    expect(existsSync(path.join(dir, 'report.jsonl'))).toBe(false);
  });
});
