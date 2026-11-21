import { describe, expect, it } from 'vitest';
import { dismissNativeReleaseBanner, shouldShowNativeReleaseBanner } from './native-release-banner';

const launch = '2026-11-25T18:00:00Z';
const start = Date.parse(launch);
const week = 7 * 24 * 60 * 60 * 1000;

describe('native release banner', () => {
  it('stays hidden without a valid launch time or before launch', () => {
    expect(shouldShowNativeReleaseBanner(undefined, null, start)).toBe(false);
    expect(shouldShowNativeReleaseBanner('', null, start)).toBe(false);
    expect(shouldShowNativeReleaseBanner('2026-11-25', null, start)).toBe(false);
    expect(shouldShowNativeReleaseBanner('2026-02-30T18:00:00Z', null, start)).toBe(false);
    expect(shouldShowNativeReleaseBanner(launch, null, start - 1)).toBe(false);
    expect(shouldShowNativeReleaseBanner(launch, null, start)).toBe(true);
  });

  it('returns seven days after dismissal, with at most three returns', () => {
    let saved: string | null = null;
    for (let appearance = 0; appearance < 4; appearance++) {
      const now = start + appearance * week;
      expect(shouldShowNativeReleaseBanner(launch, saved, now)).toBe(true);
      saved = dismissNativeReleaseBanner(saved, now);
      expect(shouldShowNativeReleaseBanner(launch, saved, now + week - 1)).toBe(false);
    }
    expect(shouldShowNativeReleaseBanner(launch, saved, start + 4 * week)).toBe(false);
  });

  it('handles a corrupt record and ignores repeat dismissals during the cooldown', () => {
    expect(shouldShowNativeReleaseBanner(launch, '{', start)).toBe(true);
    expect(shouldShowNativeReleaseBanner(launch, '{"count":-1,"dismissedAt":null}', start)).toBe(true);
    const saved = dismissNativeReleaseBanner(null, start);
    expect(dismissNativeReleaseBanner(saved, start + 1)).toBe(saved);
  });
});
