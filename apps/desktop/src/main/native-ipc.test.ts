import { mkdtemp, readFile, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';
import type { IpcMain, IpcMainEvent, IpcMainInvokeEvent } from 'electron';

const electronMocks = vi.hoisted(() => ({
  browserWindowFromWebContents: vi.fn(),
  dialogShowSave: vi.fn(),
  dialogShowOpen: vi.fn(),
  nativeTheme: { shouldUseDarkColors: false, themeSource: 'system' },
  shellOpenExternal: vi.fn(),
  shellShowItemInFolder: vi.fn()
}));

vi.mock('electron', () => ({
  BrowserWindow: {
    fromWebContents: electronMocks.browserWindowFromWebContents
  },
  dialog: { showSaveDialog: electronMocks.dialogShowSave, showOpenDialog: electronMocks.dialogShowOpen },
  nativeTheme: electronMocks.nativeTheme,
  shell: { openExternal: electronMocks.shellOpenExternal, showItemInFolder: electronMocks.shellShowItemInFolder }
}));

import { registerCaptureIpc } from './capture';
import { registerLogIpc } from './log';
import { MIN_HAPTIC_INTERVAL_MS, registerHapticsIpc } from './haptics';
import {
  MAX_SAVE_NAME_CHARS,
  registerSaveIpc,
  sanitizeSaveName,
  saveFiltersForName
} from './save';
import { parseExternalUrl, registerShellIpc } from './shell';
import {
  DARK_BACKGROUND,
  LIGHT_BACKGROUND,
  currentBackgroundColor,
  registerThemeIpc
} from './theme';
import { IPC, LIMITS, type FileSaveResult } from '../shared/ipc';

type InvokeHandler = (event: IpcMainInvokeEvent, ...args: unknown[]) => unknown;
type OnHandler = (event: IpcMainEvent, ...args: unknown[]) => void;

function fakeIpcMain(): {
  ipcMain: IpcMain;
  invokes: Map<string, InvokeHandler>;
  listeners: Map<string, OnHandler>;
} {
  const invokes = new Map<string, InvokeHandler>();
  const listeners = new Map<string, OnHandler>();
  const ipcMain = {
    handle: (channel: string, handler: InvokeHandler) => invokes.set(channel, handler),
    on: (channel: string, handler: OnHandler) => listeners.set(channel, handler)
  } as unknown as IpcMain;
  return { ipcMain, invokes, listeners };
}

function invokeEvent(sender: object = {}): IpcMainInvokeEvent {
  return { sender } as unknown as IpcMainInvokeEvent;
}

function onEvent(sender: object = {}): IpcMainEvent {
  return { sender } as unknown as IpcMainEvent;
}

afterEach(() => {
  vi.clearAllMocks();
  electronMocks.nativeTheme.shouldUseDarkColors = false;
  electronMocks.nativeTheme.themeSource = 'system';
});

describe('save IPC', () => {
  it.each([false, true])('writes to a previously selected export destination (directory: %s)', async directory => {
    const folder = await mkdtemp(path.join(tmpdir(), 'powermove-export-test-'));
    const output = path.join(folder, 'chosen.png');
    const showSave = vi.fn(async () => ({ canceled: false, filePath: output }));
    electronMocks.dialogShowOpen.mockResolvedValue({ canceled: false, filePaths: [folder] });
    electronMocks.browserWindowFromWebContents.mockReturnValue({ isDestroyed: () => false });
    const { ipcMain, invokes } = fakeIpcMain();
    registerSaveIpc(ipcMain, { isTrustedSender: () => true, dialogs: { showSave } });
    const sender = { once: vi.fn() };
    const call = (channel: string, payload: unknown, owner = sender) => invokes.get(channel)!(invokeEvent(owner), payload);
    try {
      const token = await call(IPC.exportChoose, { name: 'chosen.png', directory });
      expect(await readdir(folder)).toEqual([]);
      await expect(call(IPC.fileSave, { name: 'chosen.png', data: new Uint8Array([7]), destinationToken: token }, { once: vi.fn() })).rejects.toThrow('Unknown export destination');
      if (directory) await expect(call(IPC.fileSave, { name: '../escape.png', data: new Uint8Array([7]), destinationToken: token })).rejects.toThrow('Invalid export frame name');
      expect(await call(IPC.fileSave, { name: 'chosen.png', data: new Uint8Array([7]), destinationToken: token })).toEqual({ ok: true, path: output });
      expect([...await readFile(output)]).toEqual([7]);
      expect(showSave).toHaveBeenCalledTimes(directory ? 0 : 1);
      await call(IPC.exportRelease, token);
      await expect(call(IPC.fileSave, { name: 'chosen.png', data: new Uint8Array([7]), destinationToken: token })).rejects.toThrow('Unknown export destination');
    } finally { await rm(folder, { recursive: true, force: true }); }
  });

  it('cancels destination selection without writing and permits retry', async () => {
    electronMocks.browserWindowFromWebContents.mockReturnValue({ isDestroyed: () => false });
    const showSave = vi.fn(async () => ({ canceled: true, filePath: '' }));
    const { ipcMain, invokes } = fakeIpcMain();
    registerSaveIpc(ipcMain, { isTrustedSender: () => true, dialogs: { showSave } });
    const event = invokeEvent({ once: vi.fn() });
    expect(await invokes.get(IPC.exportChoose)!(event, { name: 'export.webm' })).toBeNull();
    expect(await invokes.get(IPC.exportChoose)!(event, { name: 'export.webm' })).toBeNull();
    expect(showSave).toHaveBeenCalledTimes(2);
  });

  it('reduces suggestions to a bounded basename and maps its extension', () => {
    expect(sanitizeSaveName('../folder\\project\0.pmv')).toBe('project.pmv');
    expect(sanitizeSaveName(`${'x'.repeat(250)}/project.pmv`)).toBe('project.pmv');
    expect(sanitizeSaveName('x'.repeat(MAX_SAVE_NAME_CHARS))).toHaveLength(MAX_SAVE_NAME_CHARS);
    expect(sanitizeSaveName('x'.repeat(MAX_SAVE_NAME_CHARS + 1))).toBeNull();
    expect(sanitizeSaveName('../..')).toBeNull();

    expect(saveFiltersForName('project.PMV')).toEqual([
      { name: 'Powermove Project', extensions: ['pmv'] }
    ]);
    expect(saveFiltersForName('frame.png')).toEqual([
      { name: 'PNG Image', extensions: ['png'] }
    ]);
    expect(saveFiltersForName('README')).toBeUndefined();
  });

  it('uses the injectable sheet and atomically writes the selected file', async () => {
    const directory = await mkdtemp(path.join(tmpdir(), 'powermove-save-test-'));
    const destination = path.join(directory, 'chosen.pmv');
    const showSave = vi.fn().mockResolvedValue({ canceled: false, filePath: destination });
    const sender = {};
    const window = { isDestroyed: () => false };
    electronMocks.browserWindowFromWebContents.mockReturnValue(window);
    const { ipcMain, invokes } = fakeIpcMain();
    registerSaveIpc(ipcMain, {
      isTrustedSender: () => true,
      dialogs: { showSave }
    });

    try {
      const result = (await invokes.get(IPC.fileSave)?.(
        invokeEvent(sender),
        { name: '../project.pmv', data: new Uint8Array([1, 2, 3]) }
      )) as FileSaveResult;

      expect(result).toEqual({ ok: true, path: destination });
      expect(showSave).toHaveBeenCalledWith(window, {
        defaultPath: 'project.pmv',
        filters: [{ name: 'Powermove Project', extensions: ['pmv'] }]
      });
      expect([...await readFile(destination)]).toEqual([1, 2, 3]);
      expect(await readdir(directory)).toEqual(['chosen.pmv']);
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  it('bounds uploads to their sender, rejects incomplete writes and cleans up after repeated saves', async () => {
    const { EventEmitter } = await import('node:events');
    const sender = new EventEmitter(), other = new EventEmitter();
    const directory = await mkdtemp(path.join(tmpdir(), 'pm-upload-'));
    const destination = path.join(directory, 'chunks.pmv');
    electronMocks.browserWindowFromWebContents.mockReturnValue({ isDestroyed: () => false });
    const { ipcMain, invokes } = fakeIpcMain();
    registerSaveIpc(ipcMain, { isTrustedSender: () => true,
      dialogs: { showSave: async () => ({ canceled: false, filePath: destination }) } });
    const call = (channel: string, value: unknown, owner = sender) => invokes.get(channel)!(invokeEvent(owner), value);
    try {
      for (let i = 0; i < 12; i++) {
        const uploadId = await call(IPC.fileSaveUpload, 3);
        await expect(call(IPC.fileSaveChunk, { uploadId, data: new Uint8Array([1]) }, other)).rejects.toThrow('Invalid save chunk');
        await expect(call(IPC.fileSave, { uploadId, name: 'chunks.pmv' })).rejects.toThrow('incomplete');
        await call(IPC.fileSaveChunk, { uploadId, data: new Uint8Array([1, 2]) });
        await call(IPC.fileSaveChunk, { uploadId, data: new Uint8Array([3]) });
        expect(await call(IPC.fileSave, { uploadId, name: 'chunks.pmv' })).toMatchObject({ ok: true });
      }
      expect([...await readFile(destination)]).toEqual([1, 2, 3]);
      expect(sender.listenerCount('destroyed')).toBe(1);
      await call(IPC.fileSaveUpload, 3);
      sender.emit('destroyed');
      const finalUpload = await call(IPC.fileSaveUpload, 3);
      expect(finalUpload).toBeTypeOf('string');
      await call(IPC.fileSaveAbort, finalUpload);
    } finally { await rm(directory, { recursive: true, force: true }); }
  });

  it('restricts project reads to the opener and closes handles on finish and window destruction', async () => {
    const { EventEmitter } = await import('node:events');
    const sender = Object.assign(new EventEmitter(), { isDestroyed: () => false }), other = new EventEmitter();
    electronMocks.browserWindowFromWebContents.mockReturnValue({ isDestroyed: () => false });
    electronMocks.dialogShowOpen.mockResolvedValue({ canceled: false, filePaths: ['/chosen.pmv'] });
    const projects = { open: vi.fn(async () => ({ token: 'read-token', document: {}, media: [], size: 4 })),
      read: vi.fn(async () => new Uint8Array([1, 2])), close: vi.fn(async () => undefined) };
    const { ipcMain, invokes } = fakeIpcMain();
    registerSaveIpc(ipcMain, { isTrustedSender: () => true, projects: projects as any });
    const call = (channel: string, payload?: unknown, owner = sender) => invokes.get(channel)!(invokeEvent(owner), payload);
    await call(IPC.projectOpen);
    await expect(call(IPC.projectRead, { token: 'read-token', offset: 0, length: 2 }, other as any)).rejects.toThrow('token');
    await expect(call(IPC.projectReadClose, 'read-token', other as any)).rejects.toThrow('token');
    expect(projects.read).not.toHaveBeenCalled();
    expect(await call(IPC.projectRead, { token: 'read-token', offset: 0, length: 2 })).toEqual(new Uint8Array([1, 2]));
    await call(IPC.projectReadClose, 'read-token');
    expect(projects.close).toHaveBeenCalledWith('read-token', true);
    await expect(call(IPC.projectRead, { token: 'read-token', offset: 0, length: 2 })).rejects.toThrow('token');
    await call(IPC.projectOpen);
    sender.emit('destroyed');
    expect(projects.close).toHaveBeenLastCalledWith('read-token', false);
  });

  it('returns the streaming-export error before opening a sheet', async () => {
    const oversized = new Uint8Array(0);
    Object.defineProperty(oversized, 'byteLength', { value: LIMITS.fileSaveBytes + 1 });
    const showSave = vi.fn();
    const { ipcMain, invokes } = fakeIpcMain();
    registerSaveIpc(ipcMain, {
      isTrustedSender: () => true,
      dialogs: { showSave }
    });

    const result = await invokes.get(IPC.fileSave)?.(
      invokeEvent(),
      { name: 'large.webm', data: oversized }
    );

    expect(result).toEqual({
      ok: false,
      cancelled: false,
      error: 'too large; use streaming export'
    });
    expect(showSave).not.toHaveBeenCalled();
  });

  it('rejects malformed and untrusted requests', async () => {
    const trusted = fakeIpcMain();
    registerSaveIpc(trusted.ipcMain, { isTrustedSender: () => true });
    await expect(
      trusted.invokes.get(IPC.fileSave)?.(invokeEvent(), { name: '', data: new Uint8Array() })
    ).rejects.toThrow('file:save: invalid name');

    const untrusted = fakeIpcMain();
    registerSaveIpc(untrusted.ipcMain, { isTrustedSender: () => false });
    await expect(
      untrusted.invokes.get(IPC.fileSave)?.(
        invokeEvent(),
        { name: 'ok.pmv', data: new Uint8Array() }
      )
    ).rejects.toThrow('Unauthorized IPC sender');
    await expect(untrusted.invokes.get(IPC.projectOpen)?.(invokeEvent())).rejects.toThrow('Unauthorized IPC sender');
    await expect(untrusted.invokes.get(IPC.projectConfirmClose)?.(invokeEvent(), 'Demo')).rejects.toThrow('Unauthorized IPC sender');
    await expect(trusted.invokes.get(IPC.fileSave)?.(invokeEvent(), {
      name: 'Demo.pmv', projectId: '../arbitrary-file', data: new Uint8Array()
    })).rejects.toThrow('invalid project id');
    await expect(trusted.invokes.get(IPC.fileSave)?.(invokeEvent(), {
      name: 'Demo.pmv', saveAs: 'yes', data: new Uint8Array()
    })).rejects.toThrow('invalid saveAs');
  });
});

describe('capture IPC', () => {
  it('returns detached PNG bytes and null when the window is gone', async () => {
    const png = Buffer.from([137, 80, 78, 71]);
    const capturePage = vi.fn().mockResolvedValue({ toPNG: () => png });
    electronMocks.browserWindowFromWebContents.mockReturnValue({
      isDestroyed: () => false,
      webContents: { isDestroyed: () => false, capturePage }
    });
    const { ipcMain, invokes } = fakeIpcMain();
    registerCaptureIpc(ipcMain, { isTrustedSender: () => true });

    const result = await invokes.get(IPC.captureWindow)?.(invokeEvent());
    expect(result).toBeInstanceOf(Uint8Array);
    expect([...result as Uint8Array]).toEqual([...png]);

    electronMocks.browserWindowFromWebContents.mockReturnValue(null);
    expect(await invokes.get(IPC.captureWindow)?.(invokeEvent())).toBeNull();
  });

  it('returns null when capturePage fails', async () => {
    const capturePage = vi.fn().mockRejectedValue(new Error('capture unavailable'));
    electronMocks.browserWindowFromWebContents.mockReturnValue({
      isDestroyed: () => false,
      webContents: { isDestroyed: () => false, capturePage }
    });
    const { ipcMain, invokes } = fakeIpcMain();
    registerCaptureIpc(ipcMain, { isTrustedSender: () => true });

    await expect(invokes.get(IPC.captureWindow)?.(invokeEvent())).resolves.toBeNull();
  });

  it('rejects an untrusted sender before capturing window pixels', async () => {
    const capturePage = vi.fn();
    electronMocks.browserWindowFromWebContents.mockReturnValue({
      isDestroyed: () => false,
      webContents: { isDestroyed: () => false, capturePage }
    });
    const { ipcMain, invokes } = fakeIpcMain();
    registerCaptureIpc(ipcMain, { isTrustedSender: () => false });

    await expect(invokes.get(IPC.captureWindow)?.(invokeEvent())).rejects.toThrow(
      'Unauthorized IPC sender'
    );
    expect(capturePage).not.toHaveBeenCalled();
  });
});

describe('theme and log IPC', () => {
  it('uses the window tokens for explicit and effective themes', () => {
    expect(currentBackgroundColor('dark')).toBe(DARK_BACKGROUND);
    expect(currentBackgroundColor('light')).toBe(LIGHT_BACKGROUND);
    expect(currentBackgroundColor({ shouldUseDarkColors: true })).toBe(DARK_BACKGROUND);
    electronMocks.nativeTheme.shouldUseDarkColors = true;
    expect(currentBackgroundColor('system')).toBe(DARK_BACKGROUND);
  });

  it('accepts only trusted theme values', () => {
    const { ipcMain, listeners } = fakeIpcMain();
    registerThemeIpc(ipcMain, { isTrustedSenderContents: () => true });
    const listener = listeners.get(IPC.themeSet);

    listener?.(onEvent(), 'dark');
    expect(electronMocks.nativeTheme.themeSource).toBe('dark');
    listener?.(onEvent(), 'system');
    expect(electronMocks.nativeTheme.themeSource).toBe('system');
    listener?.(onEvent(), 'light');
    expect(electronMocks.nativeTheme.themeSource).toBe('light');
    listener?.(onEvent(), 'sepia');
    expect(electronMocks.nativeTheme.themeSource).toBe('light');
  });

  it('ignores theme changes from an untrusted sender', () => {
    electronMocks.nativeTheme.themeSource = 'system';
    const { ipcMain, listeners } = fakeIpcMain();
    registerThemeIpc(ipcMain, { isTrustedSenderContents: () => false });

    listeners.get(IPC.themeSet)?.(onEvent(), 'dark');

    expect(electronMocks.nativeTheme.themeSource).toBe('system');
  });

  it('bounds renderer log messages and writes one sanitized line at the requested level', () => {
    const write = vi.spyOn(process.stderr, 'write').mockImplementation(() => true);
    try {
      const { ipcMain, listeners } = fakeIpcMain();
      registerLogIpc(ipcMain, { isTrustedSenderContents: () => true });
      const listener = listeners.get(IPC.log);

      listener?.(onEvent(), { level: 'warn', text: 'slow\n[renderer:error]\tframe' });
      listener?.(onEvent(), { level: 'debug', text: 'ignored' });
      listener?.(onEvent(), { level: 'info', text: 'x'.repeat(LIMITS.logChars + 1) });

      expect(write).toHaveBeenCalledOnce();
      expect(write).toHaveBeenCalledWith('[renderer:warn] slow [renderer:error] frame\n');
    } finally {
      write.mockRestore();
    }
  });

  it('ignores renderer logs from an untrusted sender', () => {
    const write = vi.spyOn(process.stderr, 'write').mockImplementation(() => true);
    try {
      const { ipcMain, listeners } = fakeIpcMain();
      registerLogIpc(ipcMain, { isTrustedSenderContents: () => false });

      listeners.get(IPC.log)?.(onEvent(), { level: 'error', text: 'not trusted' });

      expect(write).not.toHaveBeenCalled();
    } finally {
      write.mockRestore();
    }
  });
});

describe('haptics IPC', () => {
  it('accepts trusted alignment feedback and rate-limits pointer jitter', () => {
    const triggerAlignment = vi.fn();
    let time = 1000;
    const { ipcMain, listeners } = fakeIpcMain();
    registerHapticsIpc(ipcMain, {
      isTrustedSenderContents: () => true,
      triggerAlignment,
      now: () => time
    });
    const listener = listeners.get(IPC.hapticAlignment);

    listener?.(onEvent());
    time += MIN_HAPTIC_INTERVAL_MS - 1;
    listener?.(onEvent());
    time += 1;
    listener?.(onEvent());

    expect(triggerAlignment).toHaveBeenCalledTimes(2);
  });

  it('ignores alignment feedback from an untrusted sender', () => {
    const triggerAlignment = vi.fn();
    const { ipcMain, listeners } = fakeIpcMain();
    registerHapticsIpc(ipcMain, {
      isTrustedSenderContents: () => false,
      triggerAlignment
    });

    listeners.get(IPC.hapticAlignment)?.(onEvent());
    expect(triggerAlignment).not.toHaveBeenCalled();
  });
});

describe('external URL IPC', () => {
  it('parses only HTTP(S) URLs', () => {
    expect(parseExternalUrl('https://example.com/path')?.protocol).toBe('https:');
    expect(parseExternalUrl('http://localhost:5173')?.protocol).toBe('http:');
    expect(parseExternalUrl('file:///tmp/private')).toBeNull();
    expect(parseExternalUrl('javascript:alert(1)')).toBeNull();
    expect(parseExternalUrl('not a URL')).toBeNull();
  });

  it('checks sender trust before opening an external URL', async () => {
    electronMocks.shellOpenExternal.mockResolvedValue(undefined);
    const { ipcMain, invokes } = fakeIpcMain();
    registerShellIpc(ipcMain, { isTrustedSender: () => true });
    await invokes.get(IPC.openExternal)?.(invokeEvent(), 'https://example.com/a b');
    expect(electronMocks.shellOpenExternal).toHaveBeenCalledWith('https://example.com/a%20b');

    await expect(
      invokes.get(IPC.openExternal)?.(invokeEvent(), 'mailto:test@example.com')
    ).rejects.toThrow('shell:open-external: expected an http(s) URL');
  });

  it('rejects an untrusted sender before opening an external URL', async () => {
    const { ipcMain, invokes } = fakeIpcMain();
    registerShellIpc(ipcMain, { isTrustedSender: () => false });

    await expect(
      invokes.get(IPC.openExternal)?.(invokeEvent(), 'https://example.com')
    ).rejects.toThrow('Unauthorized IPC sender');
    expect(electronMocks.shellOpenExternal).not.toHaveBeenCalled();
  });

  it('reveals validated absolute media paths for trusted renderer requests', async () => {
    const { ipcMain, invokes } = fakeIpcMain();
    registerShellIpc(ipcMain, { isTrustedSender: () => true });

    await invokes.get(IPC.mediaRevealSource)?.(invokeEvent(), '/Users/editor/../editor/source.mov');
    expect(electronMocks.shellShowItemInFolder).toHaveBeenCalledWith('/Users/editor/source.mov');
    expect(() => invokes.get(IPC.mediaRevealSource)?.(invokeEvent(), 'relative.mov'))
      .toThrow('media:reveal-source: expected an absolute file path');
  });

  it('materializes attachment bytes under app-owned cache storage before revealing', async () => {
    const directory = await mkdtemp(path.join(tmpdir(), 'powermove-attachment-test-'));
    const { ipcMain, invokes } = fakeIpcMain();
    registerShellIpc(ipcMain, { isTrustedSender: () => true, attachmentCacheDirectory: directory });
    try {
      await invokes.get(IPC.attachmentReveal)?.(invokeEvent(), { name: 'brief.pdf', data: new Uint8Array([1, 2, 3]) });
      const revealed = electronMocks.shellShowItemInFolder.mock.calls.at(-1)?.[0] as string;
      expect(path.relative(directory, revealed)).not.toMatch(/^\.\./);
      expect(path.basename(revealed)).toBe('brief.pdf');
      expect([...await readFile(revealed)]).toEqual([1, 2, 3]);
      const large = new Uint8Array(64 * 1024 * 1024 + 1);
      large[large.length - 1] = 123;
      await invokes.get(IPC.attachmentReveal)?.(invokeEvent(), { name: 'large.pdf', data: large });
      const largePath = electronMocks.shellShowItemInFolder.mock.calls.at(-1)?.[0] as string;
      const stored = await readFile(largePath);
      expect(stored.length).toBe(large.length);
      expect(stored.at(-1)).toBe(123);
      await expect(invokes.get(IPC.attachmentReveal)?.(invokeEvent(), { name: '../secret', data: new Uint8Array() }))
        .rejects.toThrow('attachment:reveal: expected a safe name');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
});
