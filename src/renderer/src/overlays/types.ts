import type { PMRegistry } from '../legacy/registry';

export type OverlayPM = PMRegistry;

export type ToastOptions = {
  sticky?: boolean;
  dismissible?: boolean;
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
  onClose?: () => void;
};

export type ModalHandle = {
  el: HTMLElement;
  close(): void;
  body: HTMLElement;
};
