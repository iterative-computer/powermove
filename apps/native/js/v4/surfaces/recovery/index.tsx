/** S013 card content only. Rust owns copy, lifecycle and announcements.
 * Header/card/dock menu remain mounted by the shell. No DOM or browser APIs.
 */
import { cloneElement, h, type ComponentChildren } from "preact";
import { EmptyState, token } from "../../src/components";

export type RecoveryContent =
  | { kind: "unchanged" }
  | { kind: "hidden" }
  | { kind: "loading"; line: string }
  | {
      kind: "empty";
      title: string;
      line: string | null;
      button: string | null;
      axLabel: string;
      announcement: "high";
    }
  | { kind: "design-todo"; note: string };
export interface RecoveryProps {
  content: RecoveryContent;
  view: string;
  generation: number;
  /** Enqueues a generation-fenced host action; never synchronously enters JS. */
  restart: (request: { view: string; generation: number }) => void;
  children?: ComponentChildren;
}
export function RecoveryContentView(props: RecoveryProps) {
  const { content } = props;
  if (content.kind === "unchanged") return props.children ?? null;
  if (content.kind === "hidden" || content.kind === "design-todo") return null;
  if (content.kind === "loading") {
    return h(
      "div",
      {
        role: "group",
        label: content.line,
        style: {
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
        },
      },
      // The existing native progress primitive owns the small system spinner.
      h("div", {
        role: "progressbar",
        label: content.line,
        "data-pm-component": "progress-row",
        "data-pm-indeterminate": true,
      }),
      h(
        "text",
        {
          style: {
            fontSize: token("typography.minimum"),
            color: token("colors.muted"),
          },
        },
        content.line,
      ),
    );
  }
  const action = content.button
    ? h(
        "button",
        {
          label: content.button,
          onClick: () =>
            props.restart({ view: props.view, generation: props.generation }),
          style: {
            height: token("metrics.control"),
            minWidth: token("metrics.hit"),
            minHeight: token("metrics.hit"),
          },
        },
        content.button,
      )
    : null;
  const state = EmptyState({
    label: content.title,
    symbol: "exclamationmark.triangle",
    metadata: { pmKit: { severity: "warning" } },
    explanation: content.line ?? "",
    action,
  });
  // EmptyState builds its heading from title before replacing the AXGroup
  // label. There is one group, with exactly "{Title}. {Line}" as specified.
  return cloneElement(state, {
    label: content.axLabel,
  });
}
