/** S024/S038/S007 content recipes; copy and state come from Rust media surfaces.
 * Attach the source error group to the existing dark-token viewer scope. The
 * source bar and Back to Composition are owned by that scope and stay mounted.
 * Native NSAlert/NSOpenPanel/sequence sheets never render as JS modals here.
 */
import { h } from "preact";
import { EmptyState, ProgressRow, token } from "../../src/components";
import type { SurfaceEvent } from "../../src/protocol";

/** u64 identity fields cross the wire as decimal strings, preserving exact IDs. */
export interface Target {
  owner: {
    window: string;
    project: string;
    document: string;
    revision: string;
  };
  id: string;
  generation: string;
}
export type MediaAction = "locate" | "download" | "cancel" | "retry";
export type Dispatch = (target: Target, action: MediaAction) => void;
export interface SourceErrorModel {
  target: Target;
  title: string | null;
  line: string | null;
  button: "locate" | "download" | null;
}
export function SourceError(props: {
  model: SourceErrorModel;
  dispatch: Dispatch;
  /** Opaque SF Symbol resource supplied by the native resource provider. */
  warningSymbol: string;
  /** pm_design::Tokens.metrics.symbol_panel * 2, never a JS pixel constant. */
  warningSymbolSize: number;
}) {
  const { model, dispatch } = props;
  // Mid-playback/audio warnings use the existing viewer banner. Returning null
  // here preserves the last image/video; this view never replaces the canvas.
  if (model.title === null || model.line === null) return null;
  return h(
    "div",
    {
      role: "group",
      label: model.title,
      style: {
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
      },
    },
    h("img", {
      resource: props.warningSymbol,
      alt: "",
      role: "presentation",
      style: {
        width: props.warningSymbolSize,
        height: props.warningSymbolSize,
        color: token("colors.muted"),
      },
    }),
    h(EmptyState, {
      label: model.title,
      explanation: model.line,
      action: model.button
        ? h(
            "button",
            {
              label: model.button === "locate" ? "Locate File…" : "Download",
              onClick: () => dispatch(model.target, model.button!),
            },
            model.button === "locate" ? "Locate File…" : "Download",
          )
        : null,
    }),
  );
}
export interface TaskModel {
  target: Target;
  title: string;
  detail: string | null;
  /** Missing total is indeterminate; no fabricated percent or ETA. */
  progress: number | undefined;
  severity: "info" | "warning" | "error";
  cancel: boolean;
  retry: boolean;
  disclosure: { file: string; reason: string }[];
}
/** Body slot in the shell's Background Tasks row. Shell owns its disclosure,
 * row frame, project grouping and completed-task retirement, per promedia review.
 * This component owns no popover or window and invents no disclosure label.
 */
export function TaskBody(props: {
  model: TaskModel;
  dispatch: Dispatch;
  expanded: boolean;
}) {
  const { model, dispatch } = props;
  const colour = token(
    model.severity === "error"
      ? "colors.danger"
      : model.severity === "warning"
        ? "colors.warning"
        : "colors.text",
  );
  return h(
    "div",
    {
      role: "group",
      label: model.title,
      preventDefault: model.cancel ? { keydown: ["Escape"] } : undefined,
      onKeyDown: (event: SurfaceEvent) => {
        // Shell scopes this delivery to the focused task row. Nested fields
        // consume Escape before this handler reaches the row.
        if (event.key === "Escape" && model.cancel) {
          event.stopPropagation();
          dispatch(model.target, "cancel");
        }
      },
    },
    h("text", { style: { color: colour } }, model.title),
    model.detail
      ? h("text", { style: { color: token("colors.muted") } }, model.detail)
      : null,
    h(ProgressRow, { label: model.title, value: model.progress, children: [] }),
    model.cancel
      ? h(
          "button",
          { label: "Cancel", onClick: () => dispatch(model.target, "cancel") },
          "Cancel",
        )
      : null,
    model.retry
      ? h(
          "button",
          { label: "Retry", onClick: () => dispatch(model.target, "retry") },
          "Retry",
        )
      : null,
    props.expanded
      ? model.disclosure.map(({ file, reason }) =>
          h(
            "div",
            { key: file, role: "group", label: file },
            h("text", null, file),
            h("text", { style: { color: colour } }, reason),
          ),
        )
      : null,
  );
}
