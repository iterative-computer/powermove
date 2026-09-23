import type { OnboardingBridge } from '../../shared/ipc';

declare global {
  interface Window {
    /* No `powermove`: the kernel takes the host bridge off window at boot.
       Reach it through `bridge()` in kernel/bridge.ts. */
    onboarding: OnboardingBridge;
  }
}

export {};
