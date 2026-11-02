import { HostClient, type HostQuery } from "../../src/host-api";
import { eventDetail, type EventDetail } from "../../src/protocol";
import {
  checkFrame,
  windowRequest,
  type Intent,
  type NativeTimelineModel,
  type TimelineFrame,
  type WindowRequest,
} from "./model";

/** Worker-side lifecycle. The v4 data service invalidates the native model's
 * projection, not a JS document replica. Native inspect reads a single kernel
 * publication, avoiding incoherent project/selection/time JSON query joins.
 */
export class TimelineSession {
  private queue: Promise<unknown> = Promise.resolve();
  private cleanups: (() => Promise<void>)[] = [];
  private listeners = new Set<(frame: TimelineFrame) => void>();
  private accepting = true;
  private started = false;
  private cleanupPromise?: Promise<void>;
  private resourceRetirement?: Promise<void>;
  private serial = 0;
  private refreshPending = false;
  private window: WindowRequest;
  private current?: TimelineFrame;
  private failure: unknown;
  constructor(
    private readonly model: NativeTimelineModel,
    private readonly host: HostClient,
    window: WindowRequest,
    private readonly subscriptionPrefix: string,
    private readonly onError: (error: unknown) => void,
  ) {
    this.window = windowRequest(window.first, window.visible);
    if (!subscriptionPrefix)
      throw new TypeError("Missing timeline subscription ID prefix");
  }
  get frame(): TimelineFrame | undefined {
    return this.current;
  }
  get lastError(): unknown {
    return this.failure;
  }
  subscribe(listener: (frame: TimelineFrame) => void): () => void {
    this.listeners.add(listener);
    if (this.current) listener(this.current);
    return () => {
      this.listeners.delete(listener);
    };
  }
  private enqueue<T>(job: () => Promise<T>): Promise<T> {
    const result = this.queue.then(job);
    this.queue = result.catch(() => undefined);
    return result;
  }
  private async read(): Promise<void> {
    const frame = checkFrame(
      await this.model.inspect(this.window),
      this.window,
    );
    if (!this.accepting) return;
    this.current = frame;
    for (const listener of this.listeners) listener(frame);
  }
  private invalidate = () => {
    if (!this.accepting || this.refreshPending || !this.started) return;
    this.refreshPending = true;
    void this.enqueue(async () => {
      this.refreshPending = false;
      if (this.accepting) await this.read();
    }).catch((error) => this.report(error));
  };
  private report(error: unknown) {
    this.failure = error;
    this.onError(error);
  }
  async start(): Promise<void> {
    if (this.started || !this.accepting)
      throw new Error("Timeline session already started/retired");
    this.started = true;
    return this.enqueue(async () => {
      try {
        for (const query of [
          "project",
          "selection",
          "time",
        ] satisfies HostQuery[]) {
          const cleanup = await this.host.subscribe(
            `${this.subscriptionPrefix}:${++this.serial}:${query}`,
            query,
            this.invalidate,
          );
          this.cleanups.push(cleanup);
        }
        if (this.accepting) await this.read();
      } catch (error) {
        // Partial setup is cleaned up, and the native active gesture is cancelled.
        this.accepting = false;
        try {
          await this.cleanup();
        } catch (cleanupError) {
          throw new AggregateError(
            [error, cleanupError],
            "Timeline setup and cleanup failed",
          );
        }
        throw error;
      }
    });
  }
  async act(intent: Intent): Promise<void> {
    if (!this.started || !this.accepting)
      throw new Error("Timeline session is not active");
    // Detach caller data before queueing; preserve exact native event modifiers.
    const data = structuredClone(intent);
    if ("event" in data) data.event = eventDetail(data.event);
    const nextWindow =
      data.type === "range"
        ? windowRequest(data.first, data.visible)
        : undefined;
    return this.enqueue(async () => {
      if (!this.accepting) throw new Error("Timeline session retired");
      if (nextWindow) {
        // Range changes are projection-only, not document/model edits.
        this.window = nextWindow;
      } else {
        try {
          await this.model.act(data);
        } catch (error) {
          // Native failure feedback (e.g. a locked layer) is observable even
          // when the edit is rejected; the rejected action still rejects.
          try {
            await this.read();
          } catch (readError) {
            throw new AggregateError(
              [error, readError],
              "Timeline action and feedback failed",
            );
          }
          throw error;
        }
      }
      await this.read();
    });
  }
  /** Event callbacks cannot return promises to native. Rejections remain visible
   * through lastError/onError; no product error copy is invented here.
   */
  send = (intent: Intent): void => {
    void this.act(intent).catch((error) => this.report(error));
  };
  private cleanup(): Promise<void> {
    return (this.resourceRetirement ??= this.releaseResources());
  }
  private async releaseResources(): Promise<void> {
    const errors: unknown[] = [];
    for (const cleanup of this.cleanups.splice(0).reverse()) {
      try {
        await cleanup();
      } catch (error) {
        errors.push(error);
      }
    }
    try {
      await this.model.dispose();
    } catch (error) {
      errors.push(error);
    }
    this.listeners.clear();
    if (errors.length)
      throw new AggregateError(errors, "Timeline retirement failed");
  }
  dispose(): Promise<void> {
    if (this.cleanupPromise) return this.cleanupPromise;
    this.accepting = false; // fence queued events and late inspect/subscription results now
    this.cleanupPromise = this.enqueue(() => this.cleanup());
    return this.cleanupPromise;
  }
  async settled(): Promise<void> {
    await this.queue;
  }
}

export function nativeEvent(event: EventDetail): EventDetail {
  // SurfaceEvent has methods and routing fields which cannot cross a native ABI.
  const keys = [
    "key",
    "code",
    "repeat",
    "value",
    "shiftKey",
    "altKey",
    "metaKey",
    "ctrlKey",
    "pointerId",
    "x",
    "y",
    "button",
    "buttons",
    "clickCount",
    "deltaX",
    "deltaY",
    "selection",
    "composing",
    "composition",
    "drag",
    "range",
  ] as const;
  const detail: Record<string, unknown> = {};
  for (const key of keys)
    if (event[key] !== undefined) detail[key] = event[key];
  return eventDetail(detail as EventDetail);
}
