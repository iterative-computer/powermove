import { randomUUID } from 'node:crypto';

/** What the export's save panel (and offline message) settled. */
export type ChosenDestination = {
  file: string;
  /** The sheet format the save panel was asked for (its extension). */
  format: string;
  /** The offline message's answer, when it was asked. */
  offline: 'proxies' | 'without' | null;
};

const TTL_MS = 10 * 60_000;

/**
 * Destinations the export sheet chose before its export started: the
 * renderer asks first (offline message, save panel), shows its progress
 * sheet, prepares the project, then starts the job with the token. A token
 * is used once, by the window that chose it, for the format it was asked for.
 */
export class ExportDestinations {
  private chosen = new Map<string, ChosenDestination & { owner: number; at: number }>();

  constructor(private now: () => number = Date.now) {}

  add(owner: number, destination: ChosenDestination): string {
    this.expire();
    const token = randomUUID();
    this.chosen.set(token, { ...destination, owner, at: this.now() });
    return token;
  }

  /** The destination for `token`, removed; null when it is not this window's,
   *  was for another format, was used, or expired. */
  take(owner: number, token: unknown, format: string): ChosenDestination | null {
    this.expire();
    if (typeof token !== 'string') return null;
    const entry = this.chosen.get(token);
    if (!entry || entry.owner !== owner) return null;
    this.chosen.delete(token);
    if (entry.format !== format) return null;
    return { file: entry.file, format: entry.format, offline: entry.offline };
  }

  /** The window reloaded or closed. */
  forget(owner: number): void {
    for (const [token, entry] of this.chosen) if (entry.owner === owner) this.chosen.delete(token);
  }

  private expire(): void {
    const now = this.now();
    for (const [token, entry] of this.chosen) if (now - entry.at > TTL_MS) this.chosen.delete(token);
  }
}
