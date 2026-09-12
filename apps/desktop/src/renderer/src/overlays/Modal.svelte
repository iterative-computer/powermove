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

  onMount(() => {
    if (bodyContent instanceof HTMLElement) body.appendChild(bodyContent);
    else if (bodyContent != null) body.textContent = String(bodyContent);
    dialog.addEventListener('keydown', keydown, true);
    return () => dialog.removeEventListener('keydown', keydown, true);
  });
</script>

<div
  bind:this={dialog}
  class="modal"
  role="dialog"
  aria-modal="true"
  aria-busy={pending}
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
