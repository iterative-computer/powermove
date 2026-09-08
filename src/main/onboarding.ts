import { BrowserWindow, type IpcMain, type Rectangle } from 'electron';
import { access, mkdir, rename, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { IPC } from '../shared/ipc';
import { DARK_BACKGROUND } from './theme';

export const ONBOARDING_VERSION = 1;
export const ONBOARDING_ANIMATION_SECONDS = 9.766666666666667;
export const ONBOARDING_MARKER = `onboarding-v${ONBOARDING_VERSION}.json`;

export function onboardingEnabled(
  env: NodeJS.ProcessEnv,
  backgroundTest: boolean,
  completed: boolean
): boolean {
  if (completed) return false;
  return !backgroundTest || env['POWERMOVE_TEST_ONBOARDING'] === '1';
}

export async function onboardingCompleted(userData: string): Promise<boolean> {
  try {
    await access(path.join(userData, ONBOARDING_MARKER));
    return true;
  } catch {
    return false;
  }
}

export async function persistOnboardingCompleted(userData: string): Promise<void> {
  await mkdir(userData, { recursive: true });
  const destination = path.join(userData, ONBOARDING_MARKER);
  const temporary = `${destination}.${process.pid}.tmp`;
  await writeFile(temporary, `${JSON.stringify({ version: ONBOARDING_VERSION })}\n`, {
    encoding: 'utf8',
    mode: 0o600
  });
  await rename(temporary, destination);
}

export function onboardingOverlayOptions(
  bounds: Rectangle,
  backgroundTest: boolean
): Electron.BrowserWindowConstructorOptions {
  return {
    ...bounds,
    show: false,
    frame: false,
    transparent: true,
    backgroundColor: '#00000000',
    hasShadow: false,
    resizable: false,
    movable: false,
    minimizable: false,
    maximizable: false,
    fullscreenable: false,
    focusable: !backgroundTest,
    skipTaskbar: true,
    alwaysOnTop: !backgroundTest,
    webPreferences: {
      preload: path.join(__dirname, '../preload/onboarding.js'),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      autoplayPolicy: 'no-user-gesture-required',
      backgroundThrottling: false,
      devTools: false
    }
  };
}

export function onboardingWelcomeOptions(backgroundTest: boolean): Electron.BrowserWindowConstructorOptions {
  return {
    width: 900,
    height: 640,
    minWidth: 720,
    minHeight: 520,
    show: false,
    skipTaskbar: backgroundTest,
    focusable: !backgroundTest,
    titleBarStyle: 'hiddenInset',
    trafficLightPosition: { x: 14, y: 15 },
    backgroundColor: DARK_BACKGROUND,
    webPreferences: {
      preload: path.join(__dirname, '../preload/onboarding.js'),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      devTools: false
    }
  };
}

export interface OnboardingFlowOptions {
  appOrigin: string;
  bounds: Rectangle;
  backgroundTest: boolean;
  userData: string;
  createEditor(): BrowserWindow;
  secure(window: BrowserWindow): void;
}

export class OnboardingFlow {
  private animationWindow: BrowserWindow | null = null;
  private welcomeWindow: BrowserWindow | null = null;
  private watchdog: ReturnType<typeof setTimeout> | null = null;
  private transitioning = false;
  private handlersRegistered = false;

  constructor(
    private readonly ipc: Pick<IpcMain, 'handle' | 'on'>,
    private readonly options: OnboardingFlowOptions
  ) {}

  registerIpc(): void {
    if (this.handlersRegistered) return;
    this.handlersRegistered = true;
    this.ipc.on(IPC.onboardingAnimationComplete, (event) => {
      if (!this.isMainFrameOf(event, this.animationWindow)) return;
      void this.showWelcome();
    });
    this.ipc.on(IPC.onboardingAnimationFailed, (event, message: unknown) => {
      if (!this.isMainFrameOf(event, this.animationWindow)) return;
      console.warn('[onboarding] animation failed', String(message).slice(0, 500));
      void this.showWelcome();
    });
    this.ipc.handle(IPC.onboardingBegin, async (event) => {
      if (!this.isMainFrameOf(event, this.welcomeWindow)) {
        throw new Error('Unauthorized onboarding sender');
      }
      await this.begin();
    });
  }

  private isMainFrameOf(
    event: { sender: Electron.WebContents; senderFrame: Electron.WebFrameMain | null },
    window: BrowserWindow | null
  ): boolean {
    return !!window && !window.isDestroyed()
      && event.sender === window.webContents
      && event.senderFrame === window.webContents.mainFrame;
  }

  start(): BrowserWindow {
    this.registerIpc();
    const window = new BrowserWindow(onboardingOverlayOptions(this.options.bounds, this.options.backgroundTest));
    this.animationWindow = window;
    this.options.secure(window);
    window.setIgnoreMouseEvents(true);
    if (!this.options.backgroundTest) window.setAlwaysOnTop(true, 'screen-saver');
    window.once('ready-to-show', () => {
      if (!window.isDestroyed() && !this.options.backgroundTest) window.showInactive();
    });
    window.webContents.once('did-fail-load', () => void this.showWelcome());
    window.once('closed', () => {
      if (this.animationWindow === window) this.animationWindow = null;
    });
    void window.loadURL(`${this.options.appOrigin}/onboarding/animation.html`)
      .catch(() => this.showWelcome());
    this.watchdog = setTimeout(
      () => void this.showWelcome(),
      (ONBOARDING_ANIMATION_SECONDS + 20) * 1000
    );
    return window;
  }

  focus(): void {
    const window = this.welcomeWindow ?? this.animationWindow;
    if (!window || window.isDestroyed()) {
      void this.showWelcome();
      return;
    }
    if (this.options.backgroundTest) return;
    if (window.isMinimized()) window.restore();
    if (window === this.animationWindow) window.showInactive();
    else {
      window.show();
      window.focus();
    }
  }

  private clearWatchdog(): void {
    if (this.watchdog) clearTimeout(this.watchdog);
    this.watchdog = null;
  }

  private async showWelcome(): Promise<void> {
    if (this.transitioning || this.welcomeWindow) return;
    this.transitioning = true;
    this.clearWatchdog();
    const animation = this.animationWindow;

    const window = new BrowserWindow(onboardingWelcomeOptions(this.options.backgroundTest));
    this.welcomeWindow = window;
    this.options.secure(window);
    window.once('ready-to-show', () => {
      if (!window.isDestroyed() && !this.options.backgroundTest) {
        window.show();
        window.focus();
      }
    });
    window.once('closed', () => {
      if (this.welcomeWindow === window) this.welcomeWindow = null;
    });
    try {
      await window.loadURL(`${this.options.appOrigin}/onboarding/welcome.html`);
    } catch (error) {
      console.error('[onboarding] welcome failed to load', error);
      if (!window.isDestroyed()) window.show();
    } finally {
      this.animationWindow = null;
      if (animation && !animation.isDestroyed()) animation.destroy();
      this.transitioning = false;
    }
  }

  private async begin(): Promise<void> {
    if (this.transitioning) return;
    this.transitioning = true;
    try {
      await persistOnboardingCompleted(this.options.userData);
    } catch (error) {
      // Let the user into the editor; the welcome will return next launch if
      // persistence failed instead of trapping them on a broken disk.
      console.error('[onboarding] could not persist completion', error);
    }
    this.options.createEditor();
    const welcome = this.welcomeWindow;
    this.welcomeWindow = null;
    if (welcome && !welcome.isDestroyed()) welcome.destroy();
    this.transitioning = false;
  }
}
