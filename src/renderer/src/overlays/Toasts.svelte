<script lang="ts">
  import type { ToastOptions } from './types';

  type ToastItem = {
    id: number;
    message: string;
    error: boolean;
    sticky: boolean;
    dismissible: boolean;
    fading: boolean;
    timeout: number;
    removal: number;
  };

  let queue = $state<ToastItem[]>([]);
  let nextId = 1;

  export function push(message: unknown, milliseconds = 2200, options: ToastOptions = {}): void {
    if (message == null) return;
    const inferredError = isErrorToast(message);
    const item: ToastItem = {
      id: nextId++,
      message: String(message),
      error: inferredError,
      sticky: options.sticky ?? false,
      dismissible: options.dismissible ?? inferredError,
      fading: false,
      timeout: 0,
      removal: 0
    };
    queue.push(item);
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
    item.removal = window.setTimeout(() => dismiss(id), 260);
  }

  export function isErrorToast(message: unknown): boolean {
    return /\b(error|failed|failure|invalid|unsupported|unable)\b|could not|can(?:no|')t|larger than|stopped because/i.test(String(message));
  }
</script>

{#each queue as item (item.id)}
  <div
    class="toast"
    data-toast-id={item.id}
    data-toast-error={item.error ? 'true' : undefined}
    style:display="flex"
    style:align-items="center"
    style:gap="8px"
    style:opacity={item.fading ? '0' : undefined}
    style:transition={item.fading ? 'opacity .25s' : undefined}
  >
    <span>{item.message}</span>
    {#if item.dismissible}
      <button type="button" aria-label="Dismiss notification" onclick={() => dismiss(item.id)}>×</button>
    {/if}
  </div>
{/each}
