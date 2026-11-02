import { json, type Json } from "../../src/protocol";
import {
  checkFrame,
  type Intent,
  type NativeTimelineModel,
  type TimelineFrame,
  type WindowRequest,
} from "./model";

/** Direct worker binding to pm_panel_timeline::v4, not an invented command on
 * HostClient. The application installs this binding alongside HostDataService.
 */
export interface NativeBinding {
  window(request: WindowRequest): Promise<Json>;
  /** Calls Rust v4::dispatch with the native transport/schema/key-admission Context.
   * Resolves only after all returned HostEffects have actually completed.
   */
  dispatch(intent: Intent): Promise<void>;
  dispose(): Promise<void>;
}
/** Native schema/evaluation/media/field host decorates the bounded model window.
 * The presenter supplies only labels/readouts/leases/observed host facts, using
 * exact timeline.md copy. It must never fabricate binding handles or media facts.
 */
export interface NativePresenter {
  present(modelWindow: Json): Promise<TimelineFrame>;
}

function record(v: Json): Record<string, Json> {
  if (!v || Array.isArray(v) || typeof v !== "object")
    throw new TypeError("Invalid native model projection");
  return v;
}
/** Keep model authority even if a presentation provider accidentally supplies
 * obsolete selection/identity/lock facts. No all-document scan or local edits.
 */
export function completeFrame(
  raw: Json,
  presentation: TimelineFrame,
  request: WindowRequest,
): TimelineFrame {
  const native = record(json(raw));
  const window = record(native.window);
  if (!Array.isArray(window.rows)) throw new TypeError("Invalid native rows");
  const details = new Map(presentation.window.rows.map((row) => [row.id, row]));
  const rows = window.rows.map((value) => {
    const row = record(value);
    if (typeof row.id !== "string" || !details.has(row.id))
      throw new TypeError("Missing native row presentation");
    return { ...details.get(row.id), ...row };
  });
  // Deliberately enumerate the model envelope; unknown native row data remains
  // inspectable without becoming style or authored document writes.
  const frame = {
    ...presentation,
    document: native.document,
    revision: native.revision,
    hasComposition: native.hasComposition,
    mode: native.mode,
    view: native.view,
    snapping: native.snapping,
    indexDigits: native.indexDigits,
    selection: native.selection,
    window: { first: window.first, total: window.total, rows },
  } as unknown as TimelineFrame;
  return checkFrame(frame, request);
}
export function nativeTimeline(
  binding: NativeBinding,
  presenter: NativePresenter,
): NativeTimelineModel {
  return {
    async inspect(request) {
      const native = json(await binding.window(request));
      // Give presentation a copy so it cannot mutate model-owned facts.
      const decorated = await presenter.present(structuredClone(native));
      return completeFrame(native, decorated, request);
    },
    act(intent) {
      return binding.dispatch(intent);
    },
    dispose() {
      return binding.dispose();
    },
  };
}
