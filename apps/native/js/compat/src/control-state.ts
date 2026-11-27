import type { CompatRealm } from "./realm";
import type { Data } from "./types";

/** Connect the unchanged frozen Svelte stores to the realm's live authority.
 * The stores are read models, not a second editable project or selection.
 */
export function connectControlState(
  realm: CompatRealm,
  stores: Data,
  document: any,
  styles: readonly string[] = [],
) {
  // The playhead moving (every played frame) changes only the time: the
  // controls follow it without a copy of the whole project.
  const syncTime = () => {
    const time = Number(realm.PM.time ?? 0);
    const playing = !!realm.PM.playing;
    stores.transport.time = time;
    stores.transport.playing = playing;
    stores.updateControlTime(time, playing, performance.now(), true);
  };
  const sync = () => {
    stores.doc.replace(realm.PM.proj);
    stores.setSelection(realm.selectionSnapshot());
    syncTime();
  };
  const releases = [
    ...["project:changed", "selection", "sel"].map((name) =>
      realm.PM.bus.on(name, sync),
    ),
    ...["time", "playing"].map((name) => realm.PM.bus.on(name, syncTime)),
  ];
  const nodes = styles.map((css) => {
    const style = document.createElement("style");
    style.textContent = css;
    document.head.append(style);
    return style;
  });
  sync();
  return {
    sync,
    dispose() {
      for (const release of releases) release();
      for (const node of nodes) node.remove();
    },
  };
}
