export const NATIVE_RELEASE_BANNER_KEY = 'powermove.native-release-banner.v1';

const WEEK = 7 * 24 * 60 * 60 * 1000;
const MAX_DISMISSALS = 4; // The first appearance and up to three reminders.

function readDismissal(saved: string | null): { count: number; dismissedAt: number } {
  if (saved) {
    try {
      const record = JSON.parse(saved);
      if (Number.isInteger(record?.count) && record.count > 0 &&
          Number.isSafeInteger(record.dismissedAt) && record.dismissedAt > 0) {
        return record;
      }
    } catch {
      // A malformed record has no usable dismissal history.
    }
  }
  return { count: 0, dismissedAt: 0 };
}

export function shouldShowNativeReleaseBanner(launch: string | undefined, saved: string | null, now: number): boolean {
  if (!launch || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/.test(launch)) return false;
  const launchAt = Date.parse(launch);
  if (!Number.isFinite(launchAt) || new Date(launchAt).toISOString() !== launch.replace(/Z$/, '.000Z') || now < launchAt) {
    return false;
  }
  const { count, dismissedAt } = readDismissal(saved);
  return count < MAX_DISMISSALS && (count === 0 || now - dismissedAt >= WEEK);
}

export function dismissNativeReleaseBanner(saved: string | null, now: number): string | null {
  const { count, dismissedAt } = readDismissal(saved);
  if (count >= MAX_DISMISSALS || (count > 0 && now - dismissedAt < WEEK)) return saved;
  return JSON.stringify({ count: count + 1, dismissedAt: now });
}
