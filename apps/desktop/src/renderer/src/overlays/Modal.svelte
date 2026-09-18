<script lang="ts">
  import ErrorNotice from '../errors/ErrorNotice.svelte';
  import { onMount } from 'svelte';

  import type { ModalAction } from './types';

  let {
    title,
    bodyContent,
    actions,
    width,
    fill,
    titleId,
    onaction,
    onclose
  }: {
    title?: string;
    bodyContent?: HTMLElement | string | null;
    actions: ModalAction[];
    width: number;
    fill?: boolean;
    titleId: string;
    onaction: (index: number) => void;
    onclose: () => void;
  } = $props();

  let pending = $state(false);
  let actionError = $state<unknown>(null);
  export function setPending(value: boolean): void { pending = value; }
  export function setError(value: unknown): void { actionError = value; }

  let dialog: HTMLElement;
  let body: HTMLElement;

  export function element(): HTMLElement {
    return dialog;
  }

  export function bodyElement(): HTMLElement {
    return body;
  }

  export function focusInitial(): void {
    const focusable = focusables();
    const autofocus = dialog.querySelector<HTMLElement>('[autofocus]');
    (autofocus ?? focusable[0] ?? dialog).focus({ preventScroll: true });
  }

  function focusables(): HTMLElement[] {
    return [...dialog.querySelectorAll<HTMLElement>(
      'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
    )].filter((element) => !element.closest('[hidden], [inert]'));
  }

  function keydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      // Unlike the legacy global shortcut's blur-only field Escape, an Escape
      // anywhere inside a modal deliberately dismisses that modal uniformly.
      event.preventDefault();
      event.stopPropagation();
      onclose();
      return;
    }
    if (event.key !== 'Tab') return;
    const candidates = focusables();
    if (!candidates.length) {
      event.preventDefault();
      event.stopPropagation();
      dialog.focus({ preventScroll: true });
      return;
    }
    const current = candidates.indexOf(document.activeElement as HTMLElement);
    const next = current < 0
      ? (event.shiftKey ? candidates.length - 1 : 0)
      : (current + (event.shiftKey ? -1 : 1) + candidates.length) % candidates.length;
    event.preventDefault();
    event.stopPropagation();
    candidates[next]?.focus({ preventScroll: true });
  }

  /* The sheet follows its content: the natural height is measured whenever
     the body's children change size (rows hiding, an error appearing) and
     written as an explicit height, so CSS can ease between the two. The first
     measurement is applied before the transition is enabled, so opening does
     not animate from zero. */
  let measured = $state(false);
  let resizing = $state(false);
  let settleTimer: ReturnType<typeof setTimeout> | undefined;

  function measure(): void {
    if (!dialog?.isConnected) return;
    /* The body's own box is stretched by the height we set, so its natural
       height is the sum of its children plus padding and gaps. */
    const style = getComputedStyle(body);
    const children = [...body.children].filter((child) => !(child as HTMLElement).hidden);
    let total = parseFloat(style.paddingTop) + parseFloat(style.paddingBottom)
      + Math.max(0, children.length - 1) * (parseFloat(style.rowGap) || 0);
    if (children.length) for (const child of children) total += child.getBoundingClientRect().height;
    else total += body.scrollHeight;
    for (const part of dialog.children) {
      if (part !== body) total += (part as HTMLElement).offsetHeight;
    }
    const next = `${Math.ceil(total)}px`;
    if (dialog.style.height === next) return;
    if (measured) {
      resizing = true;
      clearTimeout(settleTimer);
      settleTimer = setTimeout(() => { resizing = false; }, 220);
    }
    dialog.style.height = next;
  }

  function settled(event: TransitionEvent): void {
    if (event.target === dialog && event.propertyName === 'height') {
      clearTimeout(settleTimer);
      resizing = false;
    }
  }

  onMount(() => {
    if (bodyContent instanceof HTMLElement) body.appendChild(bodyContent);
    else if (bodyContent != null) body.textContent = String(bodyContent);
    dialog.addEventListener('keydown', keydown, true);
    if (typeof ResizeObserver === 'undefined' || typeof MutationObserver === 'undefined') {
      return () => dialog.removeEventListener('keydown', keydown, true);
    }
    measure();
    const sizes = new ResizeObserver(() => measure());
    const watch = () => {
      sizes.disconnect();
      for (const child of body.children) sizes.observe(child);
      for (const part of dialog.children) if (part !== body) sizes.observe(part);
    };
    watch();
    const tree = new MutationObserver(() => { watch(); measure(); });
    tree.observe(dialog, { childList: true, subtree: true, attributes: true, attributeFilter: ['hidden', 'class', 'style'] });
    const enable = requestAnimationFrame(() => { measured = true; });
    return () => {
      cancelAnimationFrame(enable);
      clearTimeout(settleTimer);
      sizes.disconnect();
      tree.disconnect();
      dialog.removeEventListener('keydown', keydown, true);
    };
  });
</script>

<div
  bind:this={dialog}
  class="modal"
  class:is-measured={measured}
  class:is-resizing={resizing}
  role="dialog"
  aria-modal="true"
  aria-busy={pending}
  ontransitionend={settled}
  aria-labelledby={title ? titleId : undefined}
  aria-label={title ? undefined : 'Dialog'}
  tabindex="-1"
  style:width={`${width}px`}
  style:left="50%"
  style:top={fill ? '8vh' : '18vh'}
  style:transform="translateX(-50%)"
  style:max-height={fill ? '84vh' : '68vh'}
  data-svelte-overlay-modal="true"
>
  {#if title}<h3 id={titleId}>{title}</h3>{/if}
  <div bind:this={body} class="mb"></div>
  {#if actionError}<div class="modal-action-error"><ErrorNotice error={actionError} /></div>{/if}
  {#if actions.length}
    <div class="mf">
      {#each actions as action, index}
        <button type="button" class:btn={true} class:pri={!!action.pri} disabled={pending} onclick={() => onaction(index)}>{action.label}</button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .modal-action-error{padding:0 20px 14px}
</style>
