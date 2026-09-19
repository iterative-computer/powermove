import type { PMRegistry } from '../legacy/registry';

export type OverlayPM = PMRegistry;

/** Three families of notice, told apart by who is at fault and what it costs
    the reader. `status` is routine and passes on its own. `error` is the
    editor itself failing an operation the user asked for, and carries the
    diagnostics. `alert` is a notice raised by an extension — a refusal, a
    precondition, or the extension breaking — where the editor is intact and
    the message belongs to whoever sent it. */
export type ToastKind = 'status' | 'alert' | 'error';

export type ToastOptions = {
  /** Which family this notice belongs to. Stated here it wins over both
      `error` and the message sniffing. */
  kind?: ToastKind;
  /** The extension that raised the notice, stamped by the kernel. A notice
      with a source is never an editor error: at worst it is that extension's
      alert, and it is attributed to it by name. */
  source?: { id: string; name: string };
  /** State the outcome instead of letting the message text be sniffed. A
      success notice that quotes a file, layer or project name must stay a
      success even when that name reads like a failure ("Imported error.png"). */
  error?: boolean;
  /** Save progress from 0 to 1; null shows indeterminate activity. */
  progress?: number | null;
  completed?: boolean;
  sticky?: boolean;
  dismissible?: boolean;
  /** Prefer a Powermove icon name. When omitted, the toast message selects one. */
  icon?: string;
  /** A stable key replaces an earlier toast with the same key instead of stacking. */
  key?: string;
  /** Persistent notices sit top-right; routine status sits top-center. bottom-right is a legacy alias. */
  corner?: 'top-right' | 'bottom-right';
  /** One primary action rendered as a quiet button inside the toast. */
  action?: { label: string; run: () => void };
  /** Called when the user dismisses the toast explicitly (not on replace or timeout). */
  onDismiss?: () => void;
};

export type MenuAction = {
  icon?: string;
  label: string;
  kb?: string | null;
  on?: boolean;
  disabled?: boolean;
  curve?: readonly number[];
  run?: () => unknown;
};

export type MenuHeader = { header: string };
export type MenuItem = '-' | MenuAction | MenuHeader;

export type MenuOptions = {
  x?: number;
  y?: number;
  right?: boolean;
};

export type ModalAction = {
  label: string;
  pri?: boolean;
  run?: () => unknown;
};

export type ModalOptions = {
  title?: string;
  body?: HTMLElement | string | null;
  actions?: ModalAction[];
  width?: number;
  /** Let the dialog use the window's height; for list-shaped bodies that
      would otherwise scroll inside a short frame. */
  fill?: boolean;
  /** Operations with progress stay visible until completion or an explicit action. */
  dismissible?: boolean;
  onClose?: () => void;
};

export type ModalHandle = {
  el: HTMLElement;
  close(): void;
  body: HTMLElement;
};
