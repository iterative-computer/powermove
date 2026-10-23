export const RECONNECTING_PROGRESS = 'Connection interrupted. Reconnecting…';
export const RECONNECTED_PROGRESS = 'Connected. Continuing…';

/** Transport retries are status updates; keep their protocol details out of the UI. */
export function isConnectionRetry(message: string): boolean {
  return /^\s*Reconnecting(?:\.{3}|…)/i.test(message)
    || /^\s*Falling back from WebSockets to HTTPS transport\./i.test(message);
}
