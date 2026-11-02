import type { Json } from "../../src/protocol";
import type { Option } from "../../src/components";

export const copy = {
  heading: "Text Animators",
  add: "Add Text Animator",
  properties: "Properties",
  range: "Range",
  addProperty: "Add Animator Property",
  style: "Style Selected Text",
  selectFirst: "Select text on the canvas first",
  empty: "Add an animator to move, fade or style characters over time.",
} as const;
/** Public shipped preset ids/labels, v1.2.0 inspector/text-animators.ts.
 * Building their authored payloads belongs to the kernel text-command owner. */
export const presets = [
  ["fade", "Fade"],
  ["rise", "Rise"],
  ["rise-words", "Rise by Word"],
  ["pop", "Pop"],
  ["drop", "Spring Drop"],
  ["tumble", "Tumble"],
  ["blur", "Blur In"],
  ["tracking", "Tracking In"],
  ["typewriter", "Typewriter"],
  ["wave", "Wave"],
  ["pulse", "Pulse"],
  ["custom-stagger", "Stagger"],
  ["custom-range", "Range Selector"],
] as const;
export interface Stamp {
  project: string;
  document: number;
  revision: number;
  layer: string;
}
export interface Field {
  id: string;
  label: string;
  binding: string;
  value: Json;
  kind: "number" | "choice" | "colour";
  unit?: string;
  step?: number;
  options?: Option[];
}
export interface Animator {
  id: string;
  name: string;
  enabled: boolean;
  animated: boolean;
  /** Native leases, not raw kernel channels (ta.* is not implemented there yet). */
  enabledBinding?: string;
  nameBinding?: string;
  selector: Field[];
  properties: Field[];
  propertyCatalogue: { id: string; label: string }[];
}
export interface Publication extends Stamp {
  locked: boolean;
  animators: Animator[];
  textSelection?: { layer: string; start: number; end: number };
}
export type ActionBody =
  | { type: "preset"; preset: string }
  | { type: "style" }
  | { type: "rename"; animator: string; name: string }
  | { type: "enabled"; animator: string; value: boolean }
  | {
      type:
        | "duplicate"
        | "up"
        | "down"
        | "remove"
        | "shuffle"
        | "begin-reorder";
      animator: string;
    }
  | { type: "reorder"; animator: string; before: string }
  | { type: "begin-rename"; animator: string }
  | {
      type: "add-property" | "remove-property";
      animator: string;
      property: string;
    }
  | { type: "write"; animator: string; property: string; value: Json };
export type Action = Stamp & ActionBody;
export interface MenuItem {
  label: string;
  disabled?: boolean;
  danger?: boolean;
  separatorBefore?: boolean;
  action: ActionBody;
}
export interface EditingServices {
  /** Queue to the native host; never mutate project objects or wait on JS. */
  enqueue(action: Action): void;
  /** Host presents a real NSMenu, selection is returned to this worker queue. */
  menu(
    anchor: string,
    items: MenuItem[],
    stamp: Stamp,
    select: (action: ActionBody) => void,
  ): void;
}
export function canStyle(p: Publication): boolean {
  const s = p.textSelection;
  return !p.locked && !!s && s.layer === p.layer && s.end > s.start;
}
export function menu(p: Publication, id: string): MenuItem[] {
  const i = p.animators.findIndex((a) => a.id === id);
  if (i < 0) return [];
  const items: MenuItem[] = [
    {
      label: "Rename…",
      action: { type: "begin-rename", animator: id },
    },
    { label: "Duplicate", action: { type: "duplicate", animator: id } },
    {
      label: "Move Up",
      disabled: i === 0,
      action: { type: "up", animator: id },
    },
    {
      label: "Move Down",
      disabled: i === p.animators.length - 1,
      action: { type: "down", animator: id },
    },
    {
      label: "Remove",
      danger: true,
      separatorBefore: true,
      action: { type: "remove", animator: id },
    },
  ];
  return items.map((item) => ({
    ...item,
    disabled: p.locked || !!item.disabled,
  }));
}
export function propertyMenu(a: Animator): MenuItem[] {
  return a.propertyCatalogue
    .filter((p) => !a.properties.some((f) => f.id === p.id))
    .map((p) => ({
      label: p.label,
      action: { type: "add-property", animator: a.id, property: p.id },
    }));
}
export function dispatch(
  p: Publication,
  services: EditingServices,
  body: ActionBody,
): boolean {
  if (p.locked || (body.type === "style" && !canStyle(p))) return false;
  if ("animator" in body && !p.animators.some((a) => a.id === body.animator))
    return false;
  const { project, document, revision, layer } = p;
  services.enqueue({ project, document, revision, layer, ...body });
  return true;
}
