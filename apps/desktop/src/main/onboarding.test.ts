import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { IPC } from '../shared/ipc';

const electronMocks = vi.hoisted(() => {
  class FakeBrowserWindow {
    static windows: FakeBrowserWindow[] = [];
    readonly options: Electron.BrowserWindowConstructorOptions;
    readonly events = new Map<string, (...args: unknown[]) => void>();
    readonly contentsEvents = new Map<string, (...args: unknown[]) => void>();
    readonly webContents = {
      mainFrame: {},
      send: vi.fn(),
      once: vi.fn((name: string, listener: (...args: unknown[]) => void) => {
        this.contentsEvents.set(name, listener);
      })
    };
    destroyed = false;
    minimized = false;
    bounds: Electron.Rectangle;
    loadURL = vi.fn(async () => undefined);
    setIgnoreMouseEvents = vi.fn();
    setAlwaysOnTop = vi.fn();
    setBounds = vi.fn((bounds: Electron.Rectangle) => { this.bounds = { ...bounds }; });
    getBounds = vi.fn(() => ({ ...this.bounds }));
    getContentBounds = vi.fn(() => ({ ...this.bounds }));
    showInactive = vi.fn();
    show = vi.fn();
    focus = vi.fn();
    restore = vi.fn(() => { this.minimized = false; });

    constructor(options: Electron.BrowserWindowConstructorOptions) {
      this.options = options;
      this.bounds = {
        x: options.x ?? 0, y: options.y ?? 0,
        width: options.width ?? 800, height: options.height ?? 600
      };
      FakeBrowserWindow.windows.push(this);
    }

    once(name: string, listener: (...args: unknown[]) => void): void {
      this.events.set(name, listener);
    }

    emit(name: string): void {
      this.events.get(name)?.();
    }

    destroy(): void {
      if (this.destroyed) return;
      this.destroyed = true;
      this.emit('closed');
    }

    isDestroyed(): boolean { return this.destroyed; }
    isMinimized(): boolean { return this.minimized; }
  }
  return { FakeBrowserWindow };
});

vi.mock('electron', () => ({
  BrowserWindow: electronMocks.FakeBrowserWindow,
  nativeTheme: { shouldUseDarkColors: true, themeSource: 'system' }
}));

import {
  ONBOARDING_ANIMATION_SECONDS,
  OnboardingFlow,
  onboardingCompleted,
  onboardingEnabled,
  onboardingOverlayOptions,
  persistOnboardingCompleted
} from './onboarding';

const temporaryDirectories: string[] = [];

async function temporaryDirectory(): Promise<string> {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'powermove-onboarding-'));
  temporaryDirectories.push(directory);
  return directory;
}

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

beforeEach(() => {
  electronMocks.FakeBrowserWindow.windows.length = 0;
  vi.clearAllMocks();
});

function fakeIpc() {
  const listeners = new Map<string, (...args: any[]) => unknown>();
  const handlers = new Map<string, (...args: any[]) => unknown>();
  return {
    ipc: {
      on: vi.fn((channel: string, listener: (...args: any[]) => unknown) => {
        listeners.set(channel, listener);
      }),
      handle: vi.fn((channel: string, handler: (...args: any[]) => unknown) => {
        handlers.set(channel, handler);
      })
    },
    listeners,
    handlers
  };
}

async function settle(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
}

describe('first-run onboarding', () => {
  it('hands off an opted-in After Effects import before saving the choice', async () => {
    const userData = await temporaryDirectory();
    const { ipc, listeners, handlers } = fakeIpc();
    const editor = {} as Electron.BrowserWindow;
    let finishImport!: () => void;
    const importWorkspace = vi.fn(() => new Promise<void>(resolve => { finishImport = resolve; }));
    const flow = new OnboardingFlow(ipc as never, {
      appOrigin: 'app://powermove', displayBounds: () => ({ x: 0, y: 0, width: 1920, height: 1080 }),
      backgroundTest: true, userData, createEditor: () => editor, importWorkspace, secure: vi.fn()
    });
    const animation = flow.start() as unknown as InstanceType<typeof electronMocks.FakeBrowserWindow>;
    await listeners.get(IPC.onboardingAnimationComplete)!({ sender: animation.webContents, senderFrame: animation.webContents.mainFrame });
    await settle();
    const welcome = electronMocks.FakeBrowserWindow.windows[1]!;
    const event = { sender: welcome.webContents, senderFrame: welcome.webContents.mainFrame };
    const begin = handlers.get(IPC.onboardingBegin)!;
    await expect(begin(event, { workspaceImport: 'unsupported' })).rejects.toThrow('not supported');
    expect(importWorkspace).not.toHaveBeenCalled();
    const opening = begin(event, { workspaceImport: 'after-effects' });
    await vi.waitFor(() => expect(importWorkspace).toHaveBeenCalledExactlyOnceWith(editor, 'after-effects'));
    await begin(event, { workspaceImport: 'after-effects' });
    expect(importWorkspace).toHaveBeenCalledOnce();
    await expect(onboardingCompleted(userData)).resolves.toBe(false);
    finishImport(); await opening;
    expect(JSON.parse(await readFile(path.join(userData, 'onboarding-v1.json'), 'utf8'))).toEqual({ version: 1, workspaceImport: 'after-effects', timelineMode: 'layers', agent: null });
    expect(welcome.destroyed).toBe(true);
  });

  it('keeps onboarding retryable after a failed handoff and allows starting fresh', async () => {
    const userData = await temporaryDirectory();
    const { ipc, listeners, handlers } = fakeIpc();
    const importWorkspace = vi.fn(async () => { throw new Error('Agent not ready'); });
    const flow = new OnboardingFlow(ipc as never, {
      appOrigin: 'app://powermove', displayBounds: () => ({ x: 0, y: 0, width: 1920, height: 1080 }),
      backgroundTest: true, userData, createEditor: () => ({}) as Electron.BrowserWindow, importWorkspace, secure: vi.fn()
    });
    const animation = flow.start() as unknown as InstanceType<typeof electronMocks.FakeBrowserWindow>;
    await listeners.get(IPC.onboardingAnimationComplete)!({ sender: animation.webContents, senderFrame: animation.webContents.mainFrame });
    await settle();
    const welcome = electronMocks.FakeBrowserWindow.windows[1]!;
    const event = { sender: welcome.webContents, senderFrame: welcome.webContents.mainFrame };
    await expect(handlers.get(IPC.onboardingAppearance)!({ ...event, senderFrame: {} })).rejects.toThrow('Unauthorized');
    await expect(handlers.get(IPC.onboardingAppearance)!(event)).resolves.toBe('dark');
    const begin = handlers.get(IPC.onboardingBegin)!;
    await expect(begin(event, { workspaceImport: 'after-effects' })).rejects.toThrow('Agent not ready');
    expect(welcome.destroyed).toBe(false);
    await expect(onboardingCompleted(userData)).resolves.toBe(false);
    await begin(event, { workspaceImport: null });
    expect(importWorkspace).toHaveBeenCalledOnce();
    expect(welcome.destroyed).toBe(true);
    expect(JSON.parse(await readFile(path.join(userData, 'onboarding-v1.json'), 'utf8'))).toEqual({ version: 1, workspaceImport: null, timelineMode: 'layers', agent: null });
  });

  it('applies the chosen timeline style to the new editor and never blocks on it', async () => {
    const userData = await temporaryDirectory();
    const { ipc, listeners, handlers } = fakeIpc();
    const editor = {} as Electron.BrowserWindow;
    const applyTimelineMode = vi.fn(async () => { throw new Error('Editor still loading'); });
    const importWorkspace = vi.fn(async () => undefined);
    const flow = new OnboardingFlow(ipc as never, {
      appOrigin: 'app://powermove', displayBounds: () => ({ x: 0, y: 0, width: 1920, height: 1080 }),
      backgroundTest: true, userData, createEditor: () => editor, importWorkspace, applyTimelineMode, secure: vi.fn()
    });
    const animation = flow.start() as unknown as InstanceType<typeof electronMocks.FakeBrowserWindow>;
    await listeners.get(IPC.onboardingAnimationComplete)!({ sender: animation.webContents, senderFrame: animation.webContents.mainFrame });
    await settle();
    const welcome = electronMocks.FakeBrowserWindow.windows[1]!;
    const event = { sender: welcome.webContents, senderFrame: welcome.webContents.mainFrame };
    const begin = handlers.get(IPC.onboardingBegin)!;
    await expect(begin(event, { workspaceImport: null, timelineMode: 'storyboard' })).rejects.toThrow('timeline style');
    expect(applyTimelineMode).not.toHaveBeenCalled();
    const error = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    await begin(event, { workspaceImport: 'after-effects', timelineMode: 'tracks' });
    error.mockRestore();
    expect(applyTimelineMode).toHaveBeenCalledExactlyOnceWith(editor, 'tracks');
    expect(importWorkspace).toHaveBeenCalledOnce();
    expect(welcome.destroyed).toBe(true);
    expect(JSON.parse(await readFile(path.join(userData, 'onboarding-v1.json'), 'utf8'))).toEqual({ version: 1, workspaceImport: 'after-effects', timelineMode: 'tracks', agent: null });
  });

  it('applies the chosen agent before the import runs on it', async () => {
    const userData = await temporaryDirectory();
    const { ipc, listeners, handlers } = fakeIpc();
    const editor = {} as Electron.BrowserWindow;
    const order: string[] = [];
    const applyAgent = vi.fn(async () => { order.push('agent'); });
    const importWorkspace = vi.fn(async () => { order.push('import'); });
    const flow = new OnboardingFlow(ipc as never, {
      appOrigin: 'app://powermove', displayBounds: () => ({ x: 0, y: 0, width: 1920, height: 1080 }),
      backgroundTest: true, userData, createEditor: () => editor, importWorkspace, applyAgent, secure: vi.fn()
    });
    const animation = flow.start() as unknown as InstanceType<typeof electronMocks.FakeBrowserWindow>;
    await listeners.get(IPC.onboardingAnimationComplete)!({ sender: animation.webContents, senderFrame: animation.webContents.mainFrame });
    await settle();
    const welcome = electronMocks.FakeBrowserWindow.windows[1]!;
    const event = { sender: welcome.webContents, senderFrame: welcome.webContents.mainFrame };
    const begin = handlers.get(IPC.onboardingBegin)!;
    await expect(begin(event, { workspaceImport: null, agent: { provider: 'gemini', model: 'x' } })).rejects.toThrow('agent and model');
    await expect(begin(event, { workspaceImport: null, agent: { provider: 'claude', model: 'bad model; rm' } })).rejects.toThrow('agent and model');
    const agent = { provider: 'claude', model: 'claude-opus-5-5' };
    await begin(event, { workspaceImport: 'after-effects', agent });
    expect(applyAgent).toHaveBeenCalledExactlyOnceWith(editor, agent);
    expect(order).toEqual(['agent', 'import']);
    expect(JSON.parse(await readFile(path.join(userData, 'onboarding-v1.json'), 'utf8'))).toEqual({ version: 1, workspaceImport: 'after-effects', timelineMode: 'layers', agent });
  });

  it('skips background runs unless explicitly enabled and skips every completed profile', () => {
    expect(onboardingEnabled({}, false, false)).toBe(true);
    expect(onboardingEnabled({}, true, false)).toBe(false);
    expect(onboardingEnabled({ POWERMOVE_TEST_ONBOARDING: '1' }, true, false)).toBe(true);
    expect(onboardingEnabled({ POWERMOVE_TEST_ONBOARDING: '1' }, true, true)).toBe(false);
  });

  it('persists completion atomically only when requested and detects it on relaunch', async () => {
    const userData = await temporaryDirectory();
    await expect(onboardingCompleted(userData)).resolves.toBe(false);
    await persistOnboardingCompleted(userData);
    await expect(onboardingCompleted(userData)).resolves.toBe(true);
    const marker = JSON.parse(await readFile(path.join(userData, 'onboarding-v1.json'), 'utf8'));
    expect(marker).toEqual({ version: 1 });
  });

  it('uses a full-screen transparent frameless overlay with supplied audio autoplay enabled', () => {
    const options = onboardingOverlayOptions({ x: -1440, y: 0, width: 1440, height: 900 }, false);
    expect(options).toMatchObject({
      x: -1440, y: 0, width: 1440, height: 900,
      frame: false, transparent: true, backgroundColor: '#00000000', hasShadow: false,
      resizable: false, movable: false, skipTaskbar: true, alwaysOnTop: true,
      enableLargerThanScreen: true,
      webPreferences: {
        sandbox: true, contextIsolation: true, nodeIntegration: false,
        autoplayPolicy: 'no-user-gesture-required', backgroundThrottling: false
      }
    });
    expect(ONBOARDING_ANIMATION_SECONDS).toBeCloseTo(9.766666666666667, 12);
  });

  it('moves from animation to welcome, rejects subframe Begin, then persists after the editor is ready', async () => {
    const userData = await temporaryDirectory();
    const { ipc, listeners, handlers } = fakeIpc();
    let ready!: (window: Electron.BrowserWindow) => void;
    const createEditor = vi.fn(() => new Promise<Electron.BrowserWindow>(resolve => { ready = resolve; }));
    const flow = new OnboardingFlow(ipc as never, {
      appOrigin: 'app://powermove',
      displayBounds: () => ({ x: 0, y: 0, width: 1920, height: 1080 }),
      backgroundTest: true,
      userData,
      createEditor,
      secure: vi.fn()
    });

    const animation = flow.start() as unknown as InstanceType<typeof electronMocks.FakeBrowserWindow>;
    expect(animation.loadURL).toHaveBeenCalledWith('app://powermove/onboarding/animation.html');
    const complete = listeners.get(IPC.onboardingAnimationComplete)!;
    await complete({ sender: animation.webContents, senderFrame: animation.webContents.mainFrame });
    await settle();

    const welcome = electronMocks.FakeBrowserWindow.windows[1]!;
    expect(welcome.loadURL).toHaveBeenCalledWith('app://powermove/onboarding/welcome.html');
    expect(animation.destroyed).toBe(true);
    await expect(onboardingCompleted(userData)).resolves.toBe(false);

    const begin = handlers.get(IPC.onboardingBegin)!;
    await expect(begin({ sender: welcome.webContents, senderFrame: {} })).rejects.toThrow('Unauthorized');
    const opening = begin({ sender: welcome.webContents, senderFrame: welcome.webContents.mainFrame });
    await vi.waitFor(() => expect(createEditor).toHaveBeenCalledOnce());
    expect(welcome.destroyed).toBe(false);
    expect(flow.isFirstRunPending()).toBe(true);
    await expect(onboardingCompleted(userData)).resolves.toBe(false);
    ready({} as Electron.BrowserWindow);
    await opening;
    expect(createEditor).toHaveBeenCalledOnce();
    expect(welcome.destroyed).toBe(true);
    await expect(onboardingCompleted(userData)).resolves.toBe(true);
  });

  it('shows welcome behind the overlay at the ending cue and keeps animation alive until completion', async () => {
    const userData = await temporaryDirectory();
    const { ipc, listeners, handlers } = fakeIpc();
    const createEditor = vi.fn(() => ({}) as Electron.BrowserWindow);
    const flow = new OnboardingFlow(ipc as never, {
      appOrigin: 'app://powermove',
      displayBounds: () => ({ x: 0, y: 0, width: 1920, height: 1080 }),
      backgroundTest: true,
      userData,
      createEditor,
      secure: vi.fn()
    });

    const animation = flow.start() as unknown as InstanceType<typeof electronMocks.FakeBrowserWindow>;
    const ending = listeners.get(IPC.onboardingAnimationEnding)!;
    await ending({ sender: animation.webContents, senderFrame: {} });
    await settle();
    expect(electronMocks.FakeBrowserWindow.windows).toHaveLength(1);

    await ending({ sender: animation.webContents, senderFrame: animation.webContents.mainFrame });
    await settle();
    const welcome = electronMocks.FakeBrowserWindow.windows[1]!;
    expect(animation.destroyed).toBe(false);
    expect(welcome.loadURL).toHaveBeenCalledWith('app://powermove/onboarding/welcome.html');
    expect(welcome.options).toMatchObject({ x: 510, y: 220, width: 900, height: 640 });

    const reportTarget = listeners.get(IPC.onboardingLogoTargetReport)!;
    await reportTarget({ sender: welcome.webContents, senderFrame: welcome.webContents.mainFrame }, {
      x: 415, y: 104, width: 70, height: 58
    });
    expect(animation.webContents.send).toHaveBeenCalledWith(IPC.onboardingLogoTarget, {
      x: 925, y: 324, width: 70, height: 58
    });
    await reportTarget({ sender: welcome.webContents, senderFrame: welcome.webContents.mainFrame }, {
      x: 415, y: 104, width: Number.NaN, height: 58
    });
    expect(animation.webContents.send).toHaveBeenCalledTimes(1);

    await listeners.get(IPC.onboardingAnimationComplete)!({
      sender: animation.webContents,
      senderFrame: animation.webContents.mainFrame
    });
    await settle();
    expect(animation.destroyed).toBe(true);
    expect(welcome.destroyed).toBe(false);
    expect(electronMocks.FakeBrowserWindow.windows).toHaveLength(2);
  });

  it('cancels the overlapping animation before entering the editor from early welcome', async () => {
    const userData = await temporaryDirectory();
    const { ipc, listeners, handlers } = fakeIpc();
    const createEditor = vi.fn(() => ({}) as Electron.BrowserWindow);
    const flow = new OnboardingFlow(ipc as never, {
      appOrigin: 'app://powermove',
      displayBounds: () => ({ x: 0, y: 0, width: 1920, height: 1080 }),
      backgroundTest: true,
      userData,
      createEditor,
      secure: vi.fn()
    });
    const animation = flow.start() as unknown as InstanceType<typeof electronMocks.FakeBrowserWindow>;
    await listeners.get(IPC.onboardingAnimationEnding)!({
      sender: animation.webContents,
      senderFrame: animation.webContents.mainFrame
    });
    await settle();
    const welcome = electronMocks.FakeBrowserWindow.windows[1]!;
    await handlers.get(IPC.onboardingBegin)!({
      sender: welcome.webContents,
      senderFrame: welcome.webContents.mainFrame
    });
    expect(createEditor).toHaveBeenCalledOnce();
    expect(animation.destroyed).toBe(true);
    expect(welcome.destroyed).toBe(true);
  });

  it('queries fresh full-display bounds for every replay and enforces them after native window creation', () => {
    const displays = [
      { x: -1920, y: -24, width: 1920, height: 1080 },
      { x: 0, y: 0, width: 2560, height: 1440 }
    ];
    const displayBounds = vi.fn(() => displays.shift()!);
    const { ipc } = fakeIpc();
    const flow = new OnboardingFlow(ipc as never, {
      appOrigin: 'app://powermove', displayBounds, backgroundTest: true,
      userData: '/tmp/onboarding-test', createEditor: vi.fn(), secure: vi.fn()
    });
    const first = flow.start() as unknown as InstanceType<typeof electronMocks.FakeBrowserWindow>;
    const second = flow.replay() as unknown as InstanceType<typeof electronMocks.FakeBrowserWindow>;
    expect(displayBounds).toHaveBeenCalledTimes(2);
    expect(first.options).toMatchObject({ x: -1920, y: -24, width: 1920, height: 1080 });
    expect(first.setBounds).toHaveBeenCalledWith({ x: -1920, y: -24, width: 1920, height: 1080 }, false);
    expect(second.options).toMatchObject({ x: 0, y: 0, width: 2560, height: 1440 });
    expect(second.getBounds()).toEqual({ x: 0, y: 0, width: 2560, height: 1440 });
  });

  it('reopens an uncompleted welcome window on activation instead of bypassing onboarding', async () => {
    const userData = await temporaryDirectory();
    const { ipc, listeners } = fakeIpc();
    const createEditor = vi.fn(() => ({}) as Electron.BrowserWindow);
    const flow = new OnboardingFlow(ipc as never, {
      appOrigin: 'http://localhost:5173',
      displayBounds: () => ({ x: 0, y: 0, width: 1440, height: 900 }),
      backgroundTest: true,
      userData,
      createEditor,
      secure: vi.fn()
    });
    const animation = flow.start() as unknown as InstanceType<typeof electronMocks.FakeBrowserWindow>;
    await listeners.get(IPC.onboardingAnimationComplete)!({
      sender: animation.webContents,
      senderFrame: animation.webContents.mainFrame
    });
    await settle();
    const firstWelcome = electronMocks.FakeBrowserWindow.windows[1]!;
    firstWelcome.destroy();
    flow.focus();
    await settle();
    const reopened = electronMocks.FakeBrowserWindow.windows[2]!;
    expect(reopened.loadURL).toHaveBeenCalledWith('http://localhost:5173/onboarding/welcome.html');
    expect(createEditor).not.toHaveBeenCalled();
    await expect(onboardingCompleted(userData)).resolves.toBe(false);
  });

  it('restarts replay in one controller without layering windows and reuses the existing editor', async () => {
    const userData = await temporaryDirectory();
    const { ipc, listeners, handlers } = fakeIpc();
    const editor = {} as Electron.BrowserWindow;
    const createEditor = vi.fn(() => editor);
    const flow = new OnboardingFlow(ipc as never, {
      appOrigin: 'app://powermove',
      displayBounds: () => ({ x: 0, y: 0, width: 1920, height: 1080 }),
      backgroundTest: true,
      userData,
      createEditor,
      secure: vi.fn()
    });

    const firstAnimation = flow.start() as unknown as InstanceType<typeof electronMocks.FakeBrowserWindow>;
    const secondAnimation = flow.replay() as unknown as InstanceType<typeof electronMocks.FakeBrowserWindow>;
    expect(firstAnimation.destroyed).toBe(true);
    expect(secondAnimation.destroyed).toBe(false);
    expect(flow.hasActiveWindow()).toBe(true);

    const complete = listeners.get(IPC.onboardingAnimationComplete)!;
    await complete({ sender: firstAnimation.webContents, senderFrame: firstAnimation.webContents.mainFrame });
    await settle();
    expect(electronMocks.FakeBrowserWindow.windows).toHaveLength(2);
    await listeners.get(IPC.onboardingAnimationEnding)!({
      sender: secondAnimation.webContents,
      senderFrame: secondAnimation.webContents.mainFrame
    });
    await settle();
    const firstWelcome = electronMocks.FakeBrowserWindow.windows[2]!;
    expect(secondAnimation.destroyed).toBe(false);

    flow.replay();
    const thirdAnimation = electronMocks.FakeBrowserWindow.windows[3]!;
    expect(firstWelcome.destroyed).toBe(true);
    expect(secondAnimation.destroyed).toBe(true);
    expect(thirdAnimation.destroyed).toBe(false);
    expect(createEditor).not.toHaveBeenCalled();

    await complete({ sender: thirdAnimation.webContents, senderFrame: thirdAnimation.webContents.mainFrame });
    await settle();
    const secondWelcome = electronMocks.FakeBrowserWindow.windows[4]!;
    await handlers.get(IPC.onboardingBegin)!({
      sender: secondWelcome.webContents,
      senderFrame: secondWelcome.webContents.mainFrame
    });
    expect(createEditor).toHaveBeenCalledExactlyOnceWith();
    expect(flow.hasActiveWindow()).toBe(false);
    expect(flow.isFirstRunPending()).toBe(false);
  });
});
