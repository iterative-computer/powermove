/* Kernel-facing access to the host bridge. Only capture-bridge.ts may read
   the preload global; this module exposes its private capture to app code. */
export { bridge, provideBridge, installBridgeForTests, resetBridgeForTests } from './capture-bridge';
