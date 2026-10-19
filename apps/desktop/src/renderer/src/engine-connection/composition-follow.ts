/*
 * A viewerless (headless) engine session follows the open composition.
 *
 * Switching compositions inside a project is a document batch, not a load,
 * so nothing else tells a session that has no viewer (a viewer's
 * presentation sends its own state). The window's connection follows the
 * kernel's `comps` (compositions.open and its undo) and `project`
 * announcements, and runs `follow` before every transport command, so a
 * Play right after a switch plays the composition just opened.
 */

/** The part of a document session this needs. */
export type FollowingSession = {
  readonly closed: boolean;
  readonly headless: boolean;
  readonly composition: string | null;
  sendHeadless(composition: string): void;
};

/** The part of the kernel bus this needs. */
export type FollowBus = { on?(event: string, listener: () => void): (() => void) | void } | null | undefined;

export type CompositionFollower = {
  /** Sends the open composition to a viewerless session that shows another. */
  follow(): void;
  dispose(): void;
};

/** The announcements a composition switch makes. */
export const COMPOSITION_EVENTS = ['comps', 'project'] as const;

export function followOpenComposition(options: { session: () => FollowingSession | null; open: () => string; bus: FollowBus }): CompositionFollower {
  const follow = (): void => {
    const session = options.session();
    if (!session || session.closed || !session.headless) return;
    const composition = options.open();
    if (composition && session.composition !== composition) session.sendHeadless(composition);
  };
  const offs = COMPOSITION_EVENTS.map((event) => options.bus?.on?.(event, follow) ?? (() => {}));
  return {
    follow,
    dispose: () => {
      for (const off of offs.splice(0)) if (typeof off === 'function') off();
    }
  };
}
