/*
 * The frame-release port a window's presentation worker hands frames back
 * to the interactive engine on (engine-present/present-worker.ts,
 * main/engine/engine-process.ts). Its own module: the engine's utility
 * process imports it.
 */
/**
 * Sent (not as bytes) on a window's interactive engine port, with a
 * `MessagePort` transferred along: the engine reads that port's envelopes as
 * the window's own (a presentation worker's frame releases), so they never
 * wait for the window's main thread.
 */
export const RELEASE_PORT_OFFER = 'powermove:engine-release-port';

export type ReleasePortOffer = { type: typeof RELEASE_PORT_OFFER };

export function isReleasePortOffer(data: unknown): data is ReleasePortOffer {
  return typeof data === 'object' && data !== null && (data as { type?: unknown }).type === RELEASE_PORT_OFFER;
}
