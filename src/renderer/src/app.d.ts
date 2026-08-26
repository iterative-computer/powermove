import type { PowermoveBridge } from '../../shared/ipc';

declare global {
  interface Window {
    powermove: PowermoveBridge;
  }
}

export {};
