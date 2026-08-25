/*
 * EditGesture — the three-mode write dispatcher from legacy ui/controls.js:8-18
 * as one object, so a control cannot mismatch begin/commit.
 *
 *   command : a typed PM.Edit transaction (drag = begin/dispatch/commit; click
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

type LegacyPM = Record<string, any>;

export type EditBinding =
  | { mode: 'command'; label: string; origin?: string; command: EditCommand | ((value: unknown) => EditCommand) }
  | { mode: 'local'; label: string; set(value: unknown): void }
  | { mode: 'set'; label: string; set(value: unknown): void };

const build = (b: Extract<EditBinding, { mode: 'command' }>, value: unknown): EditCommand =>
  typeof b.command === 'function' ? b.command(value) : ({ ...b.command, value } as EditCommand);

export class EditGesture {
  constructor(private readonly PM: LegacyPM, private readonly b: EditBinding) {}

  begin(): void {
    const { PM, b } = this;
    if (b.mode === 'command') PM.Edit.begin(b.label, { origin: b.origin ?? 'interface' });
    else if (b.mode === 'set') PM.hist.begin(b.label);
  }

  write(value: unknown): void {
    const { PM, b } = this;
    if (b.mode === 'command') PM.Edit.dispatch(build(b, value));
    else b.set(value);
  }

  commit(): void {
    const { PM, b } = this;
    if (b.mode === 'command') PM.Edit.commit(b.label);
    else if (b.mode === 'set') PM.hist.commit(b.label);
  }

  cancel(): void {
    const { PM, b } = this;
    if (b.mode === 'command') PM.Edit.cancel();
    else if (b.mode === 'set') PM.hist.cancel();
  }

  /** Click-set: toggles, selects, colour apply. */
  once(value: unknown): unknown {
    const { PM, b } = this;
    if (b.mode === 'command') return PM.Edit.apply(build(b, value), { label: b.label, origin: b.origin ?? 'interface' });
    if (b.mode === 'set') return PM.hist.do(b.label, () => b.set(value));
    b.set(value);
    return undefined;
  }
}
