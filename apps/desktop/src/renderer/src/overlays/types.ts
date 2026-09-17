import type { PMRegistry } from '../legacy/registry';

export type OverlayPM = PMRegistry;

export type ToastOptions = {
  /** State the outcome instead of letting the message text be sniffed. A
      success notice that quotes a file, layer or project name must stay a
      success even when that name reads like a failure ("Imported error.png"). */
  error?: boolean;
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
  onClose?: () => void;
};

export type ModalHandle = {
  el: HTMLElement;
  close(): void;
  body: HTMLElement;
};
