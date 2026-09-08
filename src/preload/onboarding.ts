import { contextBridge, ipcRenderer } from 'electron';

import type { OnboardingBridge } from '../shared/ipc';

// Keep this preload self-contained. Electron's sandboxed preload loader cannot
// require a Rollup shared chunk, so these mirror the frozen shared IPC names.
const CHANNEL = {
  animationComplete: 'onboarding:animation-complete',
  animationFailed: 'onboarding:animation-failed',
  begin: 'onboarding:begin',
  replay: 'onboarding:replay'
} as const;

const bridge: OnboardingBridge = {
  animationComplete: () => ipcRenderer.send(CHANNEL.animationComplete),
  animationFailed: (message) => {
    ipcRenderer.send(CHANNEL.animationFailed, String(message).slice(0, 500));
  },
  begin: async () => {
    await ipcRenderer.invoke(CHANNEL.begin);
  },
  replay: async () => {
    await ipcRenderer.invoke(CHANNEL.replay);
  }
};

contextBridge.exposeInMainWorld('onboarding', bridge);
