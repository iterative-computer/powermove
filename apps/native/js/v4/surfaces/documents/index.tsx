/** S003/S004/S016/S055. All state/announcements/actions are Rust owned.
 * Native host resolves viewer material/placement metadata with pm-design.
 * No browser APIs, timers, file IO, or document mutations in these views.
 */
import type { ComponentChildren } from "preact";
import { Banner, ProgressRow, ToolbarItem, token } from "../../src/components";
export type DocumentTarget = Readonly<{
  window: number;
  project: string;
  generation: string;
}>;
export type NoticeAction = "showInFinder" | "saveAs" | "saveAgain";
export type NoticeState = Readonly<{
  text: string;
  level: "info" | "warning" | "error";
  action?: Readonly<{ id: NoticeAction; label: string }>;
}>;
type ActionHandler = (
  target: DocumentTarget,
  action: NoticeAction | "dismiss",
) => void;
const button = {
  color: token("colors.accent_text"),
  fontSize: token("typography.body"),
  minWidth: token("metrics.hit"),
  minHeight: token("metrics.hit"),
};
/** One notice is supplied by the Rust host. It owns the four-second lifetime,
 * action persistence, announcement priority and generation validation. */
export function DocumentNotice(props: {
  target: DocumentTarget;
  notice: NoticeState;
  location: "viewer" | "library";
  onAction: ActionHandler;
}) {
  const { notice, location } = props;
  const target = { ...props.target };
  return (
    <Banner
      label={notice.text}
      severity={notice.level}
      placement={location === "viewer" ? "viewer" : "inline"}
      symbol={
        {
          info: "info.circle",
          warning: "exclamationmark.triangle.fill",
          error: "xmark.octagon.fill",
        }[notice.level]
      }
      metadata={{ documentSurface: "notice", location }}
      action={
        notice.action && (
          <button
            label={notice.action.label}
            style={button}
            onClick={() => props.onAction(target, notice.action!.id)}
          >
            {notice.action.label}
          </button>
        )
      }
      onDismiss={
        notice.action ? () => props.onAction(target, "dismiss") : undefined
      }
    />
  );
}
export function DocumentLoading(props: { name: string; visible: boolean }) {
  if (!props.visible) return null;
  return (
    <div
      role="group"
      label={`Opening ${props.name}`}
      metadata={{
        documentSurface: "opening",
        placement: "viewer-frame-centre",
      }}
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <ProgressRow label={`Opening ${props.name}`}>{""}</ProgressRow>
      <text
        style={{
          fontSize: token("typography.minimum"),
          color: token("colors.muted"),
        }}
      >{`Opening “${props.name}”…`}</text>
    </div>
  );
}
/** Host moves existing content into a standard titled NSWindow. No nested card
 * or title: the window is the card. Existing trailing controls stay in order. */
export function PoppedOutPanel(props: {
  panelTitle: string;
  headerControls: ComponentChildren;
  children: ComponentChildren;
  onReturn: () => void;
}) {
  return (
    <div style={{ background: token("colors.panel") }}>
      <div
        style={{
          display: "flex",
          flexDirection: "row",
          height: token("metrics.header"),
          justifyContent: "end",
        }}
      >
        {props.headerControls}
        <ToolbarItem
          label={`Return ${props.panelTitle} to Layout`}
          title="Return to Layout"
          metadata={{
            symbol: "rectangle.inset.filled.and.arrow.left",
            borderless: true,
          }}
          style={{ symbolSize: token("metrics.symbol_panel") }}
          onClick={props.onReturn}
        />
      </div>
      {props.children}
    </div>
  );
}
