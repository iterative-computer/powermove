/*
 * EditGesture — the three-mode write dispatcher from legacy ui/controls.js:8-18
 * as one object, so a control cannot mismatch begin/commit.
 *
 *   command : a typed API edit transaction (drag = begin/dispatch/commit; click
 *             = one-shot apply). This is the mode every real control uses.
 *   local   : tool state that never touches the document (generated-panel
 *             settings).
 *   set     : legacy hist-wrapped raw mutation — migration shim only.
 *
 * The click-without-drag path (begin → cancel with zero dispatched commands)
 * is preserved on purpose: Edit.cancel() skips the project restore in that case
 * so mounted bindings keep their object refs (legacy editing.js:686-693).
 */
import type { EditCommand } from '../core/types/commands';
import type { PowermoveAPI } from '../kernel/api';

export type EditBinding =
  | { mode: 'command'; label: string; origin?: string; prepare?: () => void; command: EditCommand | ((value: unknown) => EditCommand | EditCommand[]) }
  | { mode: 'local'; label: string; set(value: unknown): void }
  | { mode: 'set'; label: string; set(value: unknown): void };

const build = (b: Extract<EditBinding, { mode: 'command' }>, value: unknown): EditCommand | EditCommand[] =>
  typeof b.command === 'function' ? b.command(value) : ({ ...b.command, value } as EditCommand);

export class EditGesture {
  constructor(private readonly api: Pick<PowermoveAPI, 'edit' | 'history'>, private readonly b: EditBinding) {}

  begin(): void {
    const { api, b } = this;
    if (b.mode === 'command') { b.prepare?.(); api.edit.begin(b.label, { origin: b.origin ?? 'interface' }); }
    else if (b.mode === 'set') api.history.begin(b.label);
  }

  write(value: unknown): void {
    const { api, b } = this;
    if (b.mode === 'command') {
      const commands = build(b, value);
      for (const command of Array.isArray(commands) ? commands : [commands]) api.edit.dispatch(command);
    }
    else b.set(value);
  }

  commit(): void {
    const { api, b } = this;
    if (b.mode === 'command') api.edit.commit(b.label);
    else if (b.mode === 'set') api.history.commit(b.label);
  }

  cancel(): void {
    const { api, b } = this;
    if (b.mode === 'command') api.edit.cancel();
    else if (b.mode === 'set') api.history.cancel();
  }

  /** Click-set: toggles, selects, colour apply. */
  once(value: unknown): unknown {
    const { api, b } = this;
    if (b.mode === 'command') { b.prepare?.(); return api.edit.apply(build(b, value), { label: b.label, origin: b.origin ?? 'interface' }); }
    if (b.mode === 'set') return api.history.do(b.label, () => b.set(value));
    b.set(value);
    return undefined;
  }
}
