<script lang="ts">
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
    fading: boolean;
    timeout: number;
    removal: number;
  };

  const LEAVE_MS = 160;

  let queue = $state<ToastItem[]>([]);
  let nextId = 1;

  export function push(message: unknown, milliseconds = 2200, options: ToastOptions = {}): void {
    if (message == null) return;
    const inferredError = isErrorToast(message);
    clear();
    const item: ToastItem = {
      id: nextId++,
      message: String(message),
      icon: options.icon || toastIcon(message, inferredError),
      error: inferredError,
      sticky: options.sticky ?? false,
      dismissible: options.dismissible ?? inferredError,
      fading: false,
      timeout: 0,
      removal: 0
    };
    queue = [item];
    if (!item.sticky) item.timeout = window.setTimeout(() => fade(item.id), milliseconds);
  }

  export function dismiss(id: number): void {
    const item = queue.find((candidate) => candidate.id === id);
    if (!item) return;
    window.clearTimeout(item.timeout);
    window.clearTimeout(item.removal);
    queue = queue.filter((candidate) => candidate.id !== id);
  }

  export function clear(): void {
    for (const item of queue) {
      window.clearTimeout(item.timeout);
      window.clearTimeout(item.removal);
    }
    queue = [];
  }

  function fade(id: number): void {
    const item = queue.find((candidate) => candidate.id === id);
    if (!item) return;
    item.fading = true;
    item.removal = window.setTimeout(() => dismiss(id), LEAVE_MS);
  }

  export function isErrorToast(message: unknown): boolean {
    return /\b(error|failed|failure|invalid|unsupported|unable)\b|could not|can(?:no|')t|larger than|stopped because/i.test(String(message));
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

{#each queue as item (item.id)}
  <div
    class="toast"
    class:leaving={item.fading}
    role={item.error ? 'alert' : 'status'}
    data-toast-id={item.id}
    data-toast-error={item.error ? 'true' : undefined}
  >
    <span class="toast-icon"><Icon {PM} name={item.icon} /></span>
    <span>{item.message}</span>
    {#if item.dismissible}
      <button type="button" aria-label="Dismiss notification" onclick={() => dismiss(item.id)}>×</button>
    {/if}
  </div>
{/each}
