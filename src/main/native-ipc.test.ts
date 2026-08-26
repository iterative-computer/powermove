import { mkdtemp, readFile, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';
import type { IpcMain, IpcMainEvent, IpcMainInvokeEvent } from 'electron';

const electronMocks = vi.hoisted(() => ({
  browserWindowFromWebContents: vi.fn(),
  dialogShowSave: vi.fn(),
  nativeTheme: { shouldUseDarkColors: false, themeSource: 'system' },
  shellOpenExternal: vi.fn()
}));

vi.mock('electron', () => ({
  BrowserWindow: {
    fromWebContents: electronMocks.browserWindowFromWebContents
  },
  dialog: { showSaveDialog: electronMocks.dialogShowSave },
  nativeTheme: electronMocks.nativeTheme,
  shell: { openExternal: electronMocks.shellOpenExternal }
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
});
