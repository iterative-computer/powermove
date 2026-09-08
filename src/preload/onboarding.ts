import { contextBridge, ipcRenderer } from 'electron';

import type { OnboardingBridge, OnboardingLogoTarget } from '../shared/ipc';

// Keep this preload self-contained. Electron's sandboxed preload loader cannot
// require a Rollup shared chunk, so these mirror the frozen shared IPC names.
const CHANNEL = {
  animationComplete: 'onboarding:animation-complete',
  animationEnding: 'onboarding:animation-ending',
  animationFailed: 'onboarding:animation-failed',
  logoTarget: 'onboarding:logo-target',
  logoTargetReport: 'onboarding:logo-target-report',
  begin: 'onboarding:begin',
  replay: 'onboarding:replay'
} as const;

const bridge: OnboardingBridge = {
  animationComplete: () => ipcRenderer.send(CHANNEL.animationComplete),
  animationEnding: () => ipcRenderer.send(CHANNEL.animationEnding),
  animationFailed: (message) => {
    ipcRenderer.send(CHANNEL.animationFailed, String(message).slice(0, 500));
  },
  reportLogoTarget: (target) => ipcRenderer.send(CHANNEL.logoTargetReport, target),
  onLogoTarget: (callback) => {
    const listener = (_event: Electron.IpcRendererEvent, target: OnboardingLogoTarget) => callback(target);
    ipcRenderer.on(CHANNEL.logoTarget, listener);
    return () => ipcRenderer.removeListener(CHANNEL.logoTarget, listener);
  },
  begin: async () => {
    await ipcRenderer.invoke(CHANNEL.begin);
  },
  replay: async () => {
    await ipcRenderer.invoke(CHANNEL.replay);
  }
};

contextBridge.exposeInMainWorld('onboarding', bridge);
