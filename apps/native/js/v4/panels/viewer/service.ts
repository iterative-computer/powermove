import type { HostClient } from "../../src/host-api";
import { json, type Json } from "../../src/protocol";
import { snapshot, type ViewerSnapshot } from "./contract";

export const viewerChannel = "viewer.chrome";
export const viewerCommand = "viewer.chrome.drive";
/** A worker client. The app must register these routes to the Rust Session.
 * No optimistic model/document mirror, local settings writes or automatic retries.
 */
export class ViewerService {
  private current: ViewerSnapshot | undefined;
  private stopped = false;
  private registration?: Promise<() => Promise<void>>;
  private stopJob?: Promise<void>;
  constructor(readonly host: HostClient) {}
  get observed() {
    return this.current && snapshot(this.current);
  }
  async start(subscription: string, changed: (state: ViewerSnapshot) => void) {
    if (this.stopped || this.registration)
      throw new Error("Viewer service lifecycle");
    this.registration = this.host.subscribe(
      subscription,
      "project",
      (value) => {
        if (this.stopped) return;
        const next = snapshot(value);
        if (next.model && next.model.project !== this.host.identity.project)
          throw new Error("Foreign viewer project");
        this.current = next;
        changed(snapshot(next));
      },
      [viewerChannel],
    );
    await this.registration;
    if (this.stopped) await this.stop();
  }
  async drive(element: string, argument: Json = null) {
    if (this.stopped || !this.current?.model)
      throw new Error("Viewer unavailable");
    return this.host.command(viewerCommand, {
      ticket: this.current.model.ticket,
      element,
      argument: json(argument),
    });
  }
  async stop() {
    this.stopped = true;
    this.stopJob ??= (async () => {
      if (!this.registration) return;
      let unsubscribe: () => Promise<void>;
      try {
        unsubscribe = await this.registration;
      } catch {
        return;
      } // HostClient already cleans up rejected registration.
      await unsubscribe();
    })();
    try {
      await this.stopJob;
    } catch (error) {
      this.stopJob = undefined;
      throw error;
    }
  }
}
