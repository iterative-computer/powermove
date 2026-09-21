<script lang="ts">
  import ToastError from '../errors/ToastError.svelte';
  import Icon from '../panels/Icon.svelte';
  import type { ToastKind, ToastOptions } from './types';

  let { PM }: { PM: Record<string, any> } = $props();

  type ToastItem = {
    progress?: number | null;
    completed?: boolean;
    pendingMessage?: string;
    id: number;
    message: string;
    icon: string;
    kind: ToastKind;
    error: boolean;
    /** The extension that spoke, shown above the message on an alert. */
    source?: string;
    sticky: boolean;
    dismissible: boolean;
    timeout: number;
    key?: string;
    corner?: 'top-right' | 'bottom-right';
    action?: { label: string; run: () => void };
    onDismiss?: () => void;
  };

  let queue = $state<ToastItem[]>([]);
  let nextId = 1;

  export function push(message: unknown, milliseconds = 2200, options: ToastOptions = {}): void {
    if (message == null) return;
    const kind = toastKind(message, options);
    const text = String(message);
    let previous: ToastItem | undefined;
    if (options.key) {
      const keyed = queue.find(item => item.key === options.key);
      if (keyed) { window.clearTimeout(keyed.timeout); previous = keyed; }
    } else {
      const existing = queue.find(item => item.message === text && item.kind === kind);
      if (existing) return;
    }
    // Routine status updates replace each other. Errors, alerts and corner notices remain available to read.
    for (const item of [...queue]) {
      if (item !== previous && item.kind === 'status' && !item.sticky && !item.corner) dismiss(item.id);
    }
    const item: ToastItem = {
      id: previous?.id ?? nextId++,
      progress: options.progress,
      completed: options.completed,
      pendingMessage: options.completed ? previous?.message : String(message),
      message: String(message),
      icon: options.icon || toastIcon(message, kind),
      kind,
      error: kind === 'error',
      source: kind === 'alert' ? options.source?.name : undefined,
      sticky: options.sticky ?? kind !== 'status',
      dismissible: options.dismissible ?? kind !== 'status',
      timeout: 0,
      key: options.key,
      corner: options.corner,
      action: options.action,
      onDismiss: options.onDismiss
    };
    queue = previous ? queue.map(old => old === previous ? item : old) : [...queue, item];
    if (!item.sticky) item.timeout = window.setTimeout(() => dismiss(item.id), milliseconds);
  }

  export function dismiss(id: number, byUser = false): void {
    const item = queue.find((candidate) => candidate.id === id);
    if (!item) return;
    window.clearTimeout(item.timeout);
    queue = queue.filter((candidate) => candidate.id !== id);
    if (byUser) item.onDismiss?.();
  }

  /* #toasts is a transformed, scrolling wrapper anchored top-center, so a
     fixed child would be positioned and clipped by it. Corner notices live
     directly under <body>. */
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy() { node.remove(); } };
  }

  const top = $derived(queue.filter(item => !item.corner));
  // Keep legacy bottom-right requests in the same top-right stack.
  const topRight = $derived(queue.filter(item => item.corner));

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

  /** Sort a notice into its family. An extension's notice never becomes an
      editor error: the editor is intact, so the worst it can be is that
      extension's alert, said in its name. */
  export function toastKind(message: unknown, options: ToastOptions = {}): ToastKind {
    if (options.kind) return options.kind;
    const failed = options.error ?? isErrorToast(message);
    if (!failed) return 'status';
    return options.source ? 'alert' : 'error';
  }

  /** A last resort for legacy string-only calls. Callers that know the outcome
      pass `error` instead, because this reads the whole message — including any
      file or project name the user chose. */
  export function isErrorToast(message: unknown): boolean {
    return message instanceof Error || /\b(error|failed|failure|invalid|unsupported|unable|ENOSPC|EACCES|EPERM|ENOENT)\b|could not|couldn[’']t|can(?:no|')t|larger than|stopped because/i.test(String(message));
  }

  /** Keep legacy string-only calls expressive without making every caller choose an icon. */
  export function toastIcon(message: unknown, kind: ToastKind = toastKind(message)): string {
    const text = String(message);
    if (kind === 'error') return 'warning';
    if (kind === 'alert') return 'caution';
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
    role={item.error ? 'alert' : 'status'}
    aria-label={item.progress !== undefined ? item.message : undefined}
    data-toast-id={item.id}
    data-toast-kind={item.kind}
    data-toast-error={item.error ? 'true' : undefined}
  >
    {#if item.error}
      <ToastError {PM} error={item.message} />
    {:else if item.kind === 'alert'}
      <span class="toast-icon"><Icon {PM} name={item.icon} /></span>
      <div class="alert-body">
        {#if item.source}<strong>{item.source}</strong>{/if}
        <span>{item.message}</span>
      </div>
    {:else}
      <span class="toast-icon save-icon" class:complete={item.completed}>
        <span class="normal-icon"><Icon {PM} name={item.icon} /></span>
        {#if item.progress !== undefined}<svg class="saved-check" aria-hidden="true" viewBox="0 0 24 24" fill="none"><path d="m5 12 4 4L19 6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" /></svg>{/if}
      </span>
      {#if item.progress !== undefined}
        <div class="save-body" class:complete={item.completed} aria-label={item.message}>
          <div class="save-labels" aria-hidden="true">
            <span class="saving-label">{item.pendingMessage || item.message}</span>
            <span class="saved-label">{item.completed ? item.message : ''}</span>
          </div>
          <div class="progress-space" aria-hidden={item.completed ? true : undefined}>
            <div class="save-progress" role="progressbar" aria-label={item.pendingMessage || item.message} aria-valuemin="0" aria-valuemax="100" aria-valuenow={item.progress === null ? undefined : Math.round(item.progress * 100)}>
              <div class="progress-fill" class:indeterminate={item.progress === null} style:transform={item.progress === null ? undefined : `scaleX(${item.progress})`}></div>
            </div>
          </div>
        </div>
      {:else}<span>{item.message}</span>{/if}
    {/if}
    {#if item.dismissible}
      {#if item.action}
        <button type="button" class="toast-action" onclick={() => { item.action?.run(); dismiss(item.id); }}>{item.action.label}</button>
      {/if}
      <button type="button" class="toast-dismiss" aria-label="Dismiss notification" title="Dismiss notification" onclick={() => dismiss(item.id, true)}><Icon {PM} name="x" /></button>
    {/if}
  </div>
{/snippet}

{#each top as item (item.id)}{@render toast(item)}{/each}
{#if topRight.length}
  <div class="toast-corner" role="status" aria-live="polite" use:portal>
    {#each topRight as item (item.id)}{@render toast(item)}{/each}
  </div>
{/if}

<style>
  .save-icon{display:grid}
  .save-icon>.normal-icon,.saved-check{grid-area:1/1;transition:opacity 240ms ease,transform 240ms ease}
  .normal-icon{display:flex}
  .saved-check{width:16px;height:16px;color:var(--success,#3fcf8e);opacity:0;transform:scale(.7)}
  .save-icon.complete .normal-icon{opacity:0;transform:scale(.7)}
  .save-icon.complete .saved-check{opacity:1;transform:scale(1)}
  .save-body{min-width:min(200px,calc(100vw - 100px));overflow-wrap:anywhere}
  .save-labels{display:grid}
  .save-labels>span{min-width:0;grid-area:1/1;transition:opacity 240ms ease,transform 240ms ease}
  .saved-label{opacity:0;transform:translateY(4px)}
  .complete .saving-label{opacity:0;transform:translateY(-4px)}
  .complete .saved-label{opacity:1;transform:translateY(0)}
  .progress-space{display:grid;grid-template-rows:1fr;opacity:1;transition:grid-template-rows 280ms ease 180ms,opacity 200ms ease 180ms}
  .complete .progress-space{grid-template-rows:0fr;opacity:0}
  .save-progress{min-height:0;overflow:hidden;position:relative}
  .save-progress::before{content:'';display:block;height:3px;margin-top:8px;background:var(--tx-2);opacity:.18;border-radius:2px}
  .progress-fill{position:absolute;bottom:0;left:0;width:100%;height:3px;border-radius:2px;background:var(--tx-1,currentColor);transform-origin:left;transition:transform 240ms ease}
  .progress-fill.indeterminate{width:35%;animation:save-activity 1.2s ease-in-out infinite}
  @keyframes save-activity{from{transform:translateX(-100%)}to{transform:translateX(290%)}}
  @media(prefers-reduced-motion:reduce){.save-labels>span,.progress-space,.progress-fill,.save-icon>.normal-icon,.saved-check{transition:none}.progress-fill.indeterminate{animation:none;transform:translateX(90%)}}
  /* An error keeps the status toast's shell and gutter and only grows
     downward, so the stack stays one column of like objects. An alert grows
     the same way, and is told apart by its marker and by the name of the
     extension speaking rather than by a shape of its own. */
  .toast[data-toast-error],
  .toast[data-toast-kind="alert"]{align-items:flex-start;min-width:min(300px,calc(100vw - 32px))}
  .toast[data-toast-error] :global(.toast-icon),
  .toast[data-toast-kind="alert"] :global(.toast-icon),
  .toast[data-toast-error]>button,
  .toast[data-toast-kind="alert"]>button{margin-top:1px}
  /* Keep the close control on the trailing edge, aligned with the first line,
     even when a short notice does not fill the minimum card width. */
  .toast>.toast-dismiss{margin-left:auto;color:var(--tx-2)}
  .toast[data-toast-error]>.toast-dismiss,
  .toast[data-toast-kind="alert"]>.toast-dismiss{margin-top:-2px}
  .toast-dismiss :global(.pm-icon){width:12px;height:12px}
  .toast>.toast-dismiss:hover{color:var(--tx)}
  .toast>.toast-dismiss:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
  .alert-body{display:flex;flex-direction:column;gap:2px;min-width:0;padding:1px 0}
  .alert-body>strong{font-weight:var(--fw-semibold);font-size:var(--fs-xs);line-height:14px;color:var(--tx-2)}
  .alert-body>span{min-width:0;line-height:1.45;white-space:pre-wrap;overflow-wrap:anywhere}
  /* The wrapper scrolls once the stack outgrows 45vh, and a scroll container
     clips at its padding edge. Pad it past the reach of --shadow-float
     (~60px below, ~40px beside). Offset the top padding so the first toast
     sits 60px from the window top, below the title bar. */
  :global(.toastwrap){box-sizing:border-box;top:28px;max-height:calc(45vh + 96px);max-width:100vw;overflow-y:auto;overscroll-behavior:contain;padding:32px 48px 64px;pointer-events:none}
  .toast{pointer-events:auto;flex-shrink:0}
</style>
