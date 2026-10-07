/**
 * Corner smoothing with Lisse (Figma's squircle curve), the same recipe the
 * desktop app's Settings uses: the radius is whatever CSS `border-radius`
 * computes to, smoothing is 0.5 on controls up to 40px tall and 0.85 above,
 * the element is clipped to the curve and re-measured on resize.
 *
 * A clip also cuts off box-shadow focus rings, so keyboard focus is drawn by
 * Lisse as an inner edge in `--squircle-focus`, and only for :focus-visible,
 * like a native control.
 */
import type { Action } from 'svelte/action';
import {
  acquirePosition,
  createSvgEffects,
  generateClipPath,
  getLayoutSize,
  observeAnchor,
  observeResize,
  releasePosition,
  type SvgEffectsHandle,
} from '@lisse/core';

const CARD_SMOOTHING = 0.85;
const CONTROL_SMOOTHING = 0.5;
const CONTROL_HEIGHT = 40;

export const squircle: Action<HTMLElement> = (el) => {
  const anchor = el.parentElement;
  const radius = parseFloat(getComputedStyle(el).borderTopLeftRadius);
  if (!anchor || !(radius > 0)) return;

  const didAcquire = acquirePosition(anchor);
  let effects: SvgEffectsHandle | undefined;
  let focus = false;

  function paint() {
    const { width, height } = getLayoutSize(el);
    if (width <= 0 || height <= 0) return;
    const smoothing = height <= CONTROL_HEIGHT ? CONTROL_SMOOTHING : CARD_SMOOTHING;
    const options = { radius: Math.min(radius, Math.min(width, height) / 2), smoothing };
    el.style.clipPath = generateClipPath(width, height, options);
    const color = getComputedStyle(el).getPropertyValue('--squircle-focus').trim();
    if (focus && color) {
      effects ??= createSvgEffects(anchor!, el);
      effects.update(options, { innerBorder: { width: 2, color, opacity: 1 } }, width, height, { x: el.offsetLeft, y: el.offsetTop });
    } else if (effects) {
      effects.destroy();
      effects = undefined;
    }
  }

  const onFocusIn = () => { focus = el.matches(':focus-visible'); paint(); };
  const onFocusOut = () => { focus = false; paint(); };
  el.addEventListener('focusin', onFocusIn);
  el.addEventListener('focusout', onFocusOut);
  // A sibling resizing moves this element without resizing it, so the anchor is watched too.
  const stopSelf = observeResize(el, paint);
  const stopAnchor = observeAnchor(anchor, el);
  el.setAttribute('data-squircle', '');
  paint();

  return {
    destroy() {
      stopSelf(); stopAnchor();
      el.removeEventListener('focusin', onFocusIn);
      el.removeEventListener('focusout', onFocusOut);
      effects?.destroy();
      if (didAcquire) releasePosition(anchor);
      el.style.clipPath = '';
      el.removeAttribute('data-squircle');
    },
  };
};
