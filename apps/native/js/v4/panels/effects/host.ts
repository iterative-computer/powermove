import { HostClient, type EditCommand } from "../../src/host-api";
import type { Json } from "../../src/protocol";
import type { EffectItem, EffectsModel, Registry, Target } from "./model";

/** Required application adapters; no guessed registry query or command is sent. */
export interface EffectsHostAdapter {
  registry: {
    subscribe(publish: (registry: Registry) => void): () => void;
  };
  target(project: Json, selection: Json): Target | null;
  add(item: EffectItem, target: Target): EditCommand[];
  reveal(item: EffectItem, target: Target): { command: string; args: Json };
}
/** Uses the public v4 worker data service; actual actor acknowledgements only. */
export class EffectsHost {
  private stopped = false;
  private stops: (() => void | Promise<void>)[] = [];
  constructor(
    readonly client: HostClient,
    readonly adapter: EffectsHostAdapter,
  ) {}
  async apply(item: EffectItem, target: Target) {
    if (this.stopped) throw new Error("Effects host retired");
    await this.client.edit(`Add ${item.label}`, this.adapter.add(item, target));
  }
  async reveal(item: EffectItem, target: Target) {
    if (this.stopped) throw new Error("Effects host retired");
    const { command, args } = this.adapter.reveal(item, target);
    await this.client.command(command, args);
  }
  async persist(key: string, value: string[]) {
    if (this.stopped) throw new Error("Effects host retired");
    await this.client.setSetting(key, value);
  }
  async connect(model: EffectsModel, subscriptionPrefix: string) {
    if (this.stopped || this.stops.length)
      throw new Error("Effects host already connected/retired");
    let project: Json | undefined, selection: Json | undefined;
    const update = () => {
      if (!this.stopped && project !== undefined && selection !== undefined)
        model.setTarget(this.adapter.target(project, selection));
    };
    const retain = async (stop: () => void | Promise<void>) => {
      if (this.stopped) {
        try {
          await stop();
        } catch (error) {
          this.stops.push(stop);
          throw error;
        }
      } else this.stops.push(stop);
    };
    try {
      await retain(
        this.adapter.registry.subscribe((registry) => {
          if (!this.stopped) model.replaceRegistry(registry);
        }),
      );
      const stamp = model.settingsStamp();
      const recent = await this.client.getSetting("pm.fx.recent");
      const collapsed = await this.client.getSetting("pm.fx.collapsed");
      if (!this.stopped)
        model.restoreSettings(recent.value, collapsed.value, stamp);
      if (this.stopped) return;
      await retain(
        await this.client.subscribe(
          `${subscriptionPrefix}:project`,
          "project",
          (value) => {
            project = value;
            update();
          },
        ),
      );
      if (this.stopped) return;
      await retain(
        await this.client.subscribe(
          `${subscriptionPrefix}:selection`,
          "selection",
          (value) => {
            selection = value;
            update();
          },
        ),
      );
    } catch (error) {
      model.setTarget(null);
      try {
        await this.dispose();
      } catch (cleanupError) {
        throw new AggregateError(
          [error, cleanupError],
          "Effects connection and cleanup failed",
        );
      }
      throw error;
    }
  }
  async dispose() {
    this.stopped = true;
    const stops = this.stops.splice(0).reverse();
    const results = await Promise.allSettled(
      stops.map((stop) => Promise.resolve().then(stop)),
    );
    const errors = results.flatMap((result, index) => {
      if (result.status !== "rejected") return [];
      this.stops.push(stops[index]);
      return [result.reason];
    });
    if (errors.length)
      throw new AggregateError(errors, "Effects subscriptions did not retire");
  }
}
