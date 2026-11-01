import type { Data } from "../../types";
export interface ExtensionHandle {
  readonly id: string;
  readonly api: Data;
  setActivating(on: boolean): void;
  registerFramePanel(definition: Data, frame: Data): { dispose(): void };
  disposeAll(): void;
}
export function createExtensionAPI(
  kernel: Data,
  record: Data,
  deps: Data,
  vars?: Readonly<Record<string, string>>,
): ExtensionHandle;
export function createVarsAPI(values?: Readonly<Record<string, string>>): {
  get(key: string): string | undefined;
  has(key: string): boolean;
  keys(): string[];
};
