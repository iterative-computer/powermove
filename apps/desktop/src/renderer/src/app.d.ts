import type { OnboardingBridge, PowermoveBridge } from '../../shared/ipc';

declare global {
  interface Window {
    powermove: PowermoveBridge;
    onboarding: OnboardingBridge;
  }
}

export {};
