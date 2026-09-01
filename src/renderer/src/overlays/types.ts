import type { PMRegistry } from '../legacy/registry';

export type OverlayPM = PMRegistry;

export type ToastOptions = {
  sticky?: boolean;
  dismissible?: boolean;
  /** Prefer a Powermove icon name. When omitted, the toast message selects one. */
  icon?: string;
};

export type MenuAction = {
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
