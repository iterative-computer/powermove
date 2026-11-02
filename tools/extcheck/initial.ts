/** Same complete empty-project factory as the frozen reference driver. */
import { installModelFactories } from "../../apps/native/js/compat/src/legacy/model";
export function initial() {
  const PM: any = {
    uid: (prefix: string) => (prefix === "P" ? "synthetic" : "synthetic-comp"),
    clamp: (v: number, a: number, b: number) => Math.max(a, Math.min(b, v)),
  };
  installModelFactories(PM);
  const document = PM.mkProject({
    name: "Synthetic extension check",
    w: 320,
    h: 180,
    fps: 30,
    dur: 2,
  });
  document.created = 0;
  return {
    revision: 0,
    document,
    selection: { layers: [], keys: [], chan: null },
    workspace: {},
    facts: {
      historyBusy: false,
      canUndo: false,
      canRedo: false,
      editLive: false,
    },
  };
}
if (import.meta.main)
  await Bun.write(process.argv[2]!, JSON.stringify(initial(), null, 2) + "\n");
