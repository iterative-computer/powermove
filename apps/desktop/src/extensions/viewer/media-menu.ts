/*
 * The viewer's preview menu (appendix D, "Viewer menu"): a button that says
 * what is on screen ("Half · Proxy"; "Full" once a paused frame upgrades)
 * and opens the app's popover menu with one radio group per section:
 *
 *   Resolution   Automatic, Full, Half, Quarter
 *   Media        Automatic, Originals, Proxies     (new engine only)
 *   ──
 *   Play Every Frame (check item)                  (new engine only)
 *
 * Resolution is the transport's preview quality, as the `<select>` it
 * replaces set it. Media and Play Every Frame are the viewer's own and go
 * to the engine in `ViewerState` (contract 2); they are kept per person in
 * the viewer's storage. "Proxies" works like Final Cut Pro's Proxy Only;
 * assets marked Always Play Original show their original in every mode.
 *
 * Built-ins import only `powermove`, so the popover menu is reached through
 * the `ui.popoverMenu` service the app registers; without it the kernel's
 * menu shows the same sections.
 */
import type { MenuContribution, PowermoveAPI } from 'powermove';
import type { FrameQuality, MediaPreference } from './engine-present/protocol';

/** The `<select>` values the transport has always taken. */
export type ResolutionChoice = 'auto' | '1' | '0.5' | '0.25';

export const RESOLUTION_ITEMS: ReadonlyArray<[ResolutionChoice, string]> = [
  ['auto', 'Automatic'], ['1', 'Full'], ['0.5', 'Half'], ['0.25', 'Quarter']
];
export const MEDIA_ITEMS: ReadonlyArray<[MediaPreference, string]> = [
  ['automatic', 'Automatic'], ['originals', 'Originals'], ['proxies', 'Proxies']
];

/** `controls/popover-menu.ts`' items, mirrored (built-ins cannot import it). */
export type PopoverItem =
  | '-'
  | { heading: string }
  | { label: string; run: () => void; checked?: boolean; check?: boolean };

export interface PopoverMenuService {
  open(req: { anchor: HTMLElement; items: PopoverItem[]; label: string; side?: 'top' | 'bottom'; align?: 'start' | 'end'; onClose?: () => void }): { close(): void };
}
export const POPOVER_MENU_SERVICE = 'ui.popoverMenu';

type Transport = Pick<PowermoveAPI['transport'], 'quality' | 'previewResolution'> & { perf: { auto: boolean } };

export function resolutionChoice(transport: Transport): ResolutionChoice {
  if (transport.perf.auto) return 'auto';
  const quality = Number(transport.quality);
  return quality <= 0.25 ? '0.25' : quality <= 0.5 ? '0.5' : '1';
}

/** Sets the preview resolution exactly as the `<select>` did. */
export function applyResolution(transport: Transport, choice: ResolutionChoice): void {
  transport.perf.auto = choice === 'auto';
  transport.quality = choice === 'auto' ? 1 : Number(choice);
  transport.previewResolution = choice;
}

const LEVEL_LABEL: Record<string, string> = { full: 'Full', half: 'Half', quarter: 'Quarter', automatic: 'Auto' };

function levelOfScale(scale: number): string {
  return scale <= 0.25 ? 'Quarter' : scale <= 0.5 ? 'Half' : 'Full';
}

/**
 * The button's label: what is on screen. With the engine presenting, the
 * frame's own quality (its resolved level and representation); otherwise
 * the current engine's preview quality.
 */
export function shownLabel(frame: FrameQuality | null | undefined, transportQuality: number): string {
  if (frame) {
    const level = LEVEL_LABEL[frame.resolution] ?? 'Full';
    const media = frame.representation === 'proxy' ? ' · Proxy' : frame.representation === 'optimizedMedia' ? ' · Optimized' : '';
    return level + media;
  }
  return levelOfScale(Number(transportQuality) || 1);
}

export type ViewerMenuState = {
  resolution: ResolutionChoice;
  media: MediaPreference;
  playEveryFrame: boolean;
  /** The new engine presents: Media and Play Every Frame apply. */
  engine: boolean;
};

export type ViewerMenuActions = {
  resolution(choice: ResolutionChoice): void;
  media(choice: MediaPreference): void;
  playEveryFrame(on: boolean): void;
};

export function viewerMenuItems(state: ViewerMenuState, act: ViewerMenuActions): PopoverItem[] {
  const items: PopoverItem[] = [{ heading: 'Resolution' }];
  for (const [value, label] of RESOLUTION_ITEMS) items.push({ label, checked: state.resolution === value, run: () => act.resolution(value) });
  if (!state.engine) return items;
  items.push({ heading: 'Media' });
  for (const [value, label] of MEDIA_ITEMS) items.push({ label, checked: state.media === value, run: () => act.media(value) });
  items.push('-', { label: 'Play Every Frame', check: true, checked: state.playEveryFrame, run: () => act.playEveryFrame(!state.playEveryFrame) });
  return items;
}

/** The same sections for the kernel's menu (`api.ui.menu`). */
export function kernelMenuItems(items: PopoverItem[]): MenuContribution[] {
  return items.map((item): MenuContribution => item === '-' ? '-'
    : 'heading' in item ? { header: item.heading }
    : { label: item.label, ...(item.checked === undefined ? {} : { on: item.checked }), run: item.run });
}

/** The keymap command: Originals ⇄ Proxies (from Automatic, to Proxies). */
export function toggledMedia(current: MediaPreference): MediaPreference {
  return current === 'proxies' ? 'originals' : 'proxies';
}

const STORAGE_MEDIA = 'media';
const STORAGE_EVERY_FRAME = 'playEveryFrame';

export type ViewerMediaPrefs = { media: MediaPreference; playEveryFrame: boolean };

export function loadMediaPrefs(storage: Pick<PowermoveAPI['storage'], 'get'> | null | undefined): ViewerMediaPrefs {
  let media: unknown, every: unknown;
  try { media = storage?.get(STORAGE_MEDIA); every = storage?.get(STORAGE_EVERY_FRAME); } catch { /* defaults */ }
  return {
    media: MEDIA_ITEMS.some(([value]) => value === media) ? media as MediaPreference : 'automatic',
    playEveryFrame: every === true
  };
}

export function saveMediaPrefs(storage: Pick<PowermoveAPI['storage'], 'set' | 'delete'> | null | undefined, prefs: ViewerMediaPrefs): void {
  try {
    if (prefs.media === 'automatic') storage?.delete(STORAGE_MEDIA); else storage?.set(STORAGE_MEDIA, prefs.media);
    if (prefs.playEveryFrame) storage?.set(STORAGE_EVERY_FRAME, true); else storage?.delete(STORAGE_EVERY_FRAME);
  } catch { /* the choice still applies to this window */ }
}

const CHEVRON = '<svg class="pm-select-chevron" viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 6.5 8 10l3.5-3.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>';

/** The menu button, drawn as the app's select trigger beside it (the zoom). */
export function createPreviewButton(style: string): { button: HTMLButtonElement; setLabel(text: string): void } {
  const button = document.createElement('button');
  button.type = 'button';
  button.id = 'preview-resolution';
  button.className = 'pm-select';
  button.setAttribute('aria-label', 'Preview quality');
  button.setAttribute('aria-haspopup', 'menu');
  button.setAttribute('aria-expanded', 'false');
  button.dataset.globalSpaceShortcut = '';
  button.style.cssText = style;
  const label = document.createElement('span');
  label.className = 'pm-select-label';
  button.append(label);
  button.insertAdjacentHTML('beforeend', CHEVRON);
  return {
    button,
    setLabel(text) {
      if (label.textContent === text) return;
      label.textContent = text;
      button.title = `Preview quality · ${text}`;
    }
  };
}
