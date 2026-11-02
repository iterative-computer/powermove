/** Synthetic fixtures only. No user project/registry/profile is captured. */
import {
  EffectsModel,
  workerSearch,
  type EffectItem,
  type ModelPorts,
} from "./model";
export const items: EffectItem[] = [
  {
    id: "blur",
    label: "Gaussian Blur",
    group: "Blur & Sharpen",
    kind: "effect",
    installed: true,
  },
  {
    id: "glow",
    label: "Glow",
    group: "Stylize",
    kind: "effect",
    installed: true,
  },
];
export function fixture(overrides: Partial<ModelPorts> = {}) {
  const applied: unknown[] = [],
    revealed: unknown[] = [],
    saved: unknown[] = [],
    errors: unknown[] = [];
  const model = new EffectsModel({
    apply: async (...args) => {
      applied.push(args);
    },
    reveal: async (...args) => {
      revealed.push(args);
    },
    persist: async (...args) => {
      saved.push(args);
    },
    report: (error) => {
      errors.push(error);
    },
    search: workerSearch,
    ...overrides,
  });
  model.replaceRegistry({ items, groups: ["Blur & Sharpen", "Stylize"] });
  model.setViewport(0, 12);
  return { model, applied, revealed, saved, errors };
}
export function itemRow(model: EffectsModel, id = "blur") {
  const row = model
    .rows()
    .find((row) => row.type === "item" && row.item.id === id);
  if (!row || row.type !== "item") throw new Error("Missing fixture item");
  return row;
}
export function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
