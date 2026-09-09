import { BrowserWindow, type IpcMain, type Rectangle } from 'electron';
import { access, mkdir, rename, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { IPC, type OnboardingLogoTarget } from '../shared/ipc';
import { LIGHT_BACKGROUND } from './theme';

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
    ...(process.platform === 'darwin' ? { type: 'panel' as const } : {}),
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
    enableLargerThanScreen: true,
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

export function onboardingWelcomeOptions(
  backgroundTest: boolean,
  displayBounds?: Rectangle
): Electron.BrowserWindowConstructorOptions {
  const width = 900;
  const height = 640;
  return {
    width,
    height,
    ...(displayBounds ? {
      x: Math.round(displayBounds.x + (displayBounds.width - width) / 2),
      y: Math.round(displayBounds.y + (displayBounds.height - height) / 2)
    } : {}),
    minWidth: 720,
    minHeight: 520,
    show: false,
    skipTaskbar: backgroundTest,
    focusable: !backgroundTest,
    titleBarStyle: 'hiddenInset',
    trafficLightPosition: { x: 14, y: 15 },
    backgroundColor: LIGHT_BACKGROUND,
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
  displayBounds(): Rectangle;
  backgroundTest: boolean;
  userData: string;
  createEditor(): BrowserWindow | Promise<BrowserWindow>;
  secure(window: BrowserWindow): void;
}

export class OnboardingFlow {
  private animationWindow: BrowserWindow | null = null;
  private welcomeWindow: BrowserWindow | null = null;
  private watchdog: ReturnType<typeof setTimeout> | null = null;
  private transitioning = false;
  private handlersRegistered = false;
  private generation = 0;
  private firstRunPending = false;
  private currentDisplayBounds: Rectangle | null = null;

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
    this.ipc.on(IPC.onboardingAnimationEnding, (event) => {
      if (!this.isMainFrameOf(event, this.animationWindow)) return;
      void this.showWelcome(this.generation, true);
    });
    this.ipc.on(IPC.onboardingAnimationFailed, (event, message: unknown) => {
      if (!this.isMainFrameOf(event, this.animationWindow)) return;
      console.warn('[onboarding] animation failed', String(message).slice(0, 500));
      void this.showWelcome();
    });
    this.ipc.on(IPC.onboardingLogoTargetReport, (event, target: unknown) => {
      if (!this.isMainFrameOf(event, this.welcomeWindow)) return;
      const converted = this.convertLogoTarget(target);
      const animation = this.animationWindow;
      if (!converted || !animation || animation.isDestroyed()) return;
      animation.webContents.send(IPC.onboardingLogoTarget, converted);
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
    this.firstRunPending = true;
    return this.replay();
  }

  isFirstRunPending(): boolean {
    return this.firstRunPending;
  }

  hasActiveWindow(): boolean {
    return !!((this.animationWindow && !this.animationWindow.isDestroyed())
      || (this.welcomeWindow && !this.welcomeWindow.isDestroyed()));
  }

  replay(): BrowserWindow {
    this.registerIpc();
    const generation = ++this.generation;
    this.clearWatchdog();
    this.transitioning = false;
    const previousAnimation = this.animationWindow;
    const previousWelcome = this.welcomeWindow;
    this.animationWindow = null;
    this.welcomeWindow = null;

    const bounds = this.options.displayBounds();
    this.currentDisplayBounds = { ...bounds };
    const window = new BrowserWindow(onboardingOverlayOptions(bounds, this.options.backgroundTest));
    this.animationWindow = window;
    this.options.secure(window);
    window.setIgnoreMouseEvents(true);
    if (!this.options.backgroundTest) window.setAlwaysOnTop(true, 'screen-saver');
    this.applyOverlayBounds(window, bounds);
    window.once('ready-to-show', () => {
      if (!window.isDestroyed() && !this.options.backgroundTest) {
        if (!this.applyOverlayBounds(window, bounds)) console.warn('[onboarding] overlay bounds were constrained', window.getBounds(), bounds);
        window.showInactive();
      }
    });
    window.webContents.once('did-fail-load', () => void this.showWelcome(generation));
    window.once('closed', () => {
      if (this.animationWindow === window) this.animationWindow = null;
    });
    void window.loadURL(`${this.options.appOrigin}/onboarding/animation.html`)
      .catch(() => this.showWelcome(generation));
    this.watchdog = setTimeout(
      () => void this.showWelcome(generation),
      (ONBOARDING_ANIMATION_SECONDS + 20) * 1000
    );
    if (previousAnimation && !previousAnimation.isDestroyed()) previousAnimation.destroy();
    if (previousWelcome && !previousWelcome.isDestroyed()) previousWelcome.destroy();
    return window;
  }

  focus(): void {
    const window = this.welcomeWindow ?? this.animationWindow;
    if (!window || window.isDestroyed()) {
      void this.showWelcome(this.generation);
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

  private applyOverlayBounds(window: BrowserWindow, bounds: Rectangle): boolean {
    window.setBounds(bounds, false);
    const actual = window.getBounds();
    return actual.x === bounds.x && actual.y === bounds.y
      && actual.width === bounds.width && actual.height === bounds.height;
  }

  private finishAnimation(generation: number): void {
    if (generation !== this.generation) return;
    this.clearWatchdog();
    const animation = this.animationWindow;
    this.animationWindow = null;
    if (animation && !animation.isDestroyed()) animation.destroy();
  }

  private convertLogoTarget(target: unknown): OnboardingLogoTarget | null {
    const welcome = this.welcomeWindow;
    const animation = this.animationWindow;
    if (!welcome || welcome.isDestroyed() || !animation || animation.isDestroyed()
      || !target || typeof target !== 'object') return null;
    const candidate = target as Record<string, unknown>;
    const values = ['x', 'y', 'width', 'height'].map((key) => Number(candidate[key]));
    if (!values.every(Number.isFinite)) return null;
    const [x, y, width, height] = values as [number, number, number, number];
    const welcomeContent = welcome.getContentBounds();
    if (x < 0 || y < 0 || width <= 0 || height <= 0 || width > 512 || height > 512
      || x + width > welcomeContent.width + 1 || y + height > welcomeContent.height + 1) return null;
    const overlayContent = animation.getContentBounds();
    const converted = {
      x: welcomeContent.x + x - overlayContent.x,
      y: welcomeContent.y + y - overlayContent.y,
      width,
      height
    };
    if (converted.x < -1 || converted.y < -1
      || converted.x + width > overlayContent.width + 1
      || converted.y + height > overlayContent.height + 1) return null;
    return converted;
  }

  private async showWelcome(generation = this.generation, preserveAnimation = false): Promise<void> {
    if (generation !== this.generation) return;
    if (this.welcomeWindow) {
      if (!preserveAnimation) this.finishAnimation(generation);
      return;
    }
    if (this.transitioning) {
      if (!preserveAnimation) this.finishAnimation(generation);
      return;
    }
    this.transitioning = true;

    const window = new BrowserWindow(onboardingWelcomeOptions(
      this.options.backgroundTest,
      this.currentDisplayBounds ?? undefined
    ));
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
      if (generation !== this.generation) {
        if (!window.isDestroyed()) window.destroy();
        return;
      }
      if (!preserveAnimation) this.finishAnimation(generation);
      this.transitioning = false;
    }
  }

  private async begin(): Promise<void> {
    if (this.transitioning) return;
    const generation = this.generation;
    this.transitioning = true;
    try {
      await persistOnboardingCompleted(this.options.userData);
    } catch (error) {
      // Let the user into the editor; the welcome will return next launch if
      // persistence failed instead of trapping them on a broken disk.
      console.error('[onboarding] could not persist completion', error);
    }
    // A native Replay command can arrive while the marker write is pending.
    // Leave that newer animation and its audio/window lifecycle untouched.
    if (generation !== this.generation) return;
    try {
      await this.options.createEditor();
    } catch (error) {
      this.transitioning = false;
      throw error;
    }
    this.firstRunPending = false;
    this.finishAnimation(generation);
    const welcome = this.welcomeWindow;
    this.welcomeWindow = null;
    if (welcome && !welcome.isDestroyed()) welcome.destroy();
    this.transitioning = false;
  }
}
