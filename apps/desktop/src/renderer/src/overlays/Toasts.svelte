<script lang="ts">
  import ErrorNotice from '../errors/ErrorNotice.svelte';
  import Icon from '../panels/Icon.svelte';
  import type { ToastOptions } from './types';

  let { PM }: { PM: Record<string, any> } = $props();

  type ToastItem = {
    id: number;
    message: string;
    icon: string;
    error: boolean;
    sticky: boolean;
    dismissible: boolean;
    timeout: number;
    key?: string;
    corner?: 'top-right';
    action?: { label: string; run: () => void };
    onDismiss?: () => void;
  };

  let queue = $state<ToastItem[]>([]);
  let nextId = 1;

  export function push(message: unknown, milliseconds = 2200, options: ToastOptions = {}): void {
    if (message == null) return;
    const inferredError = isErrorToast(message);
    const text = String(message);
    if (options.key) {
      const keyed = queue.find(item => item.key === options.key);
      if (keyed) { window.clearTimeout(keyed.timeout); queue = queue.filter(item => item !== keyed); }
    } else {
      const existing = queue.find(item => item.message === text && item.error === inferredError);
      if (existing) return;
    }
    // Routine status updates replace each other. Errors and corner notices remain available to read.
    for (const item of [...queue]) {
      if (!item.error && !item.sticky && !item.corner) dismiss(item.id);
    }
    const item: ToastItem = {
      id: nextId++,
      message: String(message),
      icon: options.icon || toastIcon(message, inferredError),
      error: inferredError,
      sticky: options.sticky ?? inferredError,
      dismissible: options.dismissible ?? inferredError,
      timeout: 0,
      key: options.key,
      corner: options.corner,
      action: options.action,
      onDismiss: options.onDismiss
    };
    queue = [...queue, item];
    if (!item.sticky) item.timeout = window.setTimeout(() => dismiss(item.id), milliseconds);
  }

  export function dismiss(id: number, byUser = false): void {
    const item = queue.find((candidate) => candidate.id === id);
    if (!item) return;
    window.clearTimeout(item.timeout);
    queue = queue.filter((candidate) => candidate.id !== id);
    if (byUser) item.onDismiss?.();
  }

  /* #toasts is a transformed, scrolling wrapper anchored bottom-center, so a
     fixed child would be positioned and clipped by it. Corner notices live
     directly under <body>. */
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy() { node.remove(); } };
  }

  const bottom = $derived(queue.filter(item => !item.corner));
  const corner = $derived(queue.filter(item => item.corner === 'top-right'));

  /** Remove a keyed notice without treating it as a user dismissal. */
  export function dismissKey(key: string): void {
    const item = queue.find((candidate) => candidate.key === key);
    if (item) dismiss(item.id);
  }

  export function clear(): void {
    for (const item of queue) {
      window.clearTimeout(item.timeout);
    }
    queue = [];
  }

  export function isErrorToast(message: unknown): boolean {
    return message instanceof Error || /\b(error|failed|failure|invalid|unsupported|unable|ENOSPC|EACCES|EPERM|ENOENT)\b|could not|couldn[’']t|can(?:no|')t|larger than|stopped because/i.test(String(message));
  }

  /** Keep legacy string-only calls expressive without making every caller choose an icon. */
  export function toastIcon(message: unknown, error = isErrorToast(message)): string {
    const text = String(message);
    if (error) return 'x';
    if (/^undo\b|\bundone\b/i.test(text)) return 'undo';
    if (/^redo\b|\bredone\b/i.test(text)) return 'redo';
    if (/\b(delet(?:e|ed)|trash(?:ed)?|removed?)\b/i.test(text)) return 'trash';
    if (/\b(save|saved|download|export(?:ed)?)\b/i.test(text)) return 'export';
    if (/\b(open(?:ed)?|project|workspace|composition)\b/i.test(text)) return 'project';
    if (/\b(copy|copied|paste|pasted|layer|layers)\b/i.test(text)) return 'layers';
    if (/\b(import(?:ed|ing)?|add(?:ed)?|creat(?:e|ed)|insert(?:ed)?)\b/i.test(text)) return 'plus';
    if (/\b(move|moved|drop|drag)\b/i.test(text)) return 'hand';
    if (/\b(select|selected|pointer)\b/i.test(text)) return 'cursor';
    if (/\b(preview|visible|shown?)\b/i.test(text)) return 'eye';
    if (/\b(audio|sound)\b/i.test(text)) return 'music';
    if (/\b(video|movie|footage)\b/i.test(text)) return 'film';
    if (/\b(image|photo|picture)\b/i.test(text)) return 'image';
    if (/\b(panel|layout)\b/i.test(text)) return 'panel';
    if (/\b(effect|agent|generat(?:e|ed)|appl(?:y|ied)|updated?)\b/i.test(text)) return 'sparkle';
    return 'note';
  }
</script>

{#snippet toast(item: ToastItem)}
  <div
    class="toast"
    role={item.error ? undefined : 'status'}
    data-toast-id={item.id}
    data-toast-error={item.error ? 'true' : undefined}
  >
    {#if item.error}
      <ErrorNotice error={item.message} />
    {:else}
      <span class="toast-icon"><Icon {PM} name={item.icon} /></span>
      <span>{item.message}</span>
    {/if}
    {#if item.dismissible}
      {#if item.action}
        <button type="button" class="toast-action" onclick={() => { item.action?.run(); dismiss(item.id); }}>{item.action.label}</button>
      {/if}
      <button type="button" aria-label="Dismiss notification" onclick={() => dismiss(item.id, true)}>×</button>
    {/if}
  </div>
{/snippet}

{#each bottom as item (item.id)}{@render toast(item)}{/each}
{#if corner.length}
  <div class="toast-corner" role="status" aria-live="polite" use:portal>
    {#each corner as item (item.id)}{@render toast(item)}{/each}
  </div>
{/if}

<style>
  .toast[data-toast-error]{align-items:flex-start;padding:0 6px 0 0;width:min(440px,calc(100vw - 32px));gap:0}
  .toast[data-toast-error] :global(.error-notice){border:0;background:transparent}
  .toast[data-toast-error]>button{margin-top:8px}
  /* The wrapper scrolls once the stack outgrows 45vh, and a scroll container
     clips at its padding edge. Pad it past the reach of --shadow-float
     (~60px below, ~40px beside) and pull the anchor down by the same amount
     so the toast itself sits where it always did. */
  :global(.toastwrap){box-sizing:border-box;bottom:-28px;max-height:calc(45vh + 96px);max-width:100vw;overflow-y:auto;overscroll-behavior:contain;padding:32px 48px 64px;pointer-events:none}
  .toast{pointer-events:auto;flex-shrink:0}
</style>
