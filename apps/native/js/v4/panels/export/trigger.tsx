import { useLayoutEffect, useReducer } from "preact/hooks";
import { ToolbarItem } from "../../src/components";
import { ExportModel, isProgress } from "./model";
/** Shell keeps the registered command/menu and shortcut arbitration. */
export function ExportButton({ model }: { model: ExportModel }) {
  const [, update] = useReducer((n) => n + 1, 0);
  useLayoutEffect(() => model.subscribe(() => update(null)), [model]);
  const state = model.state;
  const running =
    isProgress(state.run) || state.run.phase === "choosingDestination";
  return (
    <ToolbarItem
      id="export.toolbar"
      label="Export"
      title={running ? "Export already running" : "Export (⌘E)"}
      disabled={!state.project || running}
      onClick={() => {
        void model.show().catch(() => update(null));
      }}
    />
  );
}
