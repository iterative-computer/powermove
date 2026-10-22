<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { bridge } from '../../kernel/bridge';
  import Icon from '../Icon.svelte';
  import ErrorNotice from '../../errors/ErrorNotice.svelte';
  import { readVisualReply, visualReplyEvent, type VisualReply } from '../../../../shared/visual-reply';
  import { prepareVisualReply, visualDocument } from './visual-reply';
  let { artifact, PM }: { artifact: VisualReply; PM: Record<string, any> } = $props();
  let iframe = $state<HTMLIFrameElement>();
  let dialog: HTMLDialogElement;
  let closeButton = $state<HTMLButtonElement>();
  let expandButton = $state<HTMLButtonElement>();
  let expanded = $state(false), zoom = $state(1), replay = $state(0);
  let height = $state(360), ready = $state(false), error = $state('');
  let modalShadow = $state('');
  let token = '';
  let documentHtml = $state('');
  let timeout: ReturnType<typeof setTimeout>;
  function theme() {
    const styles = getComputedStyle(document.documentElement);
    // Expanded chrome uses the app-level modal material, outside the Agent
    // panel's deliberately lighter local popover shadow.
    modalShadow = styles.getPropertyValue('--shadow-float').trim();
    if (!iframe?.contentWindow) return;
    iframe.contentWindow.postMessage({ channel: 'powermove-visual-theme', token,
      dark: document.documentElement.dataset.theme === 'dark', colors: {
        '--background': styles.getPropertyValue('--bg-panel').trim() || styles.getPropertyValue('--bg-2').trim() || '#202022',
        '--foreground': styles.getPropertyValue('--tx').trim() || '#eeeeee',
        '--muted': styles.getPropertyValue('--tx-3').trim() || '#999999',
        '--accent': styles.getPropertyValue('--accent').trim() || '#a7a0ff',
      } }, '*');
  }
  function initialize() {
    iframe?.contentWindow?.postMessage({ channel: 'powermove-visual-init', html: documentHtml }, '*');
  }
  $effect(() => {
    void replay;
    ready = false; error = '';
    token = crypto.randomUUID();
    clearTimeout(timeout);
    try {
      if (!bridge()?.versions.electron) throw new Error('Open this visual reply in the Powermove desktop app.');
      if (!readVisualReply(artifact)) throw new Error('This saved visualization is invalid or no longer supported. Ask the agent to regenerate it.');
      const input = prepareVisualReply({ title: artifact.title, html: artifact.html, height: artifact.height });
      documentHtml = visualDocument(input.html, token); height = input.height;
      timeout = setTimeout(() => { if (!ready) error = 'The visualization did not start. Replay it or ask the agent to fix it.'; }, 8000);
    } catch (e) { documentHtml = ''; error = (e as Error).message; }
    return () => clearTimeout(timeout);
  });
  onMount(() => {
    theme();
    // Native pointer-focus handling can leave focus on the document after a
    // toolbar click. The open modal still owns Escape in that case.
    const escape = (event: KeyboardEvent) => {
      if (!expanded || !dialog?.open || event.key !== 'Escape') return;
      event.preventDefault(); event.stopImmediatePropagation(); close();
    };
    const receive = (event: MessageEvent) => {
      const data = visualReplyEvent(event, iframe?.contentWindow ?? null, token);
      if (!data) return;
      if (data.type === 'ready') { ready = true; clearTimeout(timeout); theme(); }
      if (data.type === 'size' && data.height) height = Math.min(artifact.height, data.height);
      if (data.type === 'error') error = data.message || 'The visualization could not load.';
    };
    window.addEventListener('message', receive);
    window.addEventListener('keydown', escape, true);
    const watch = new MutationObserver(theme);
    watch.observe(document.documentElement, { attributes: true, attributeFilter: ['style','class','data-theme'] });
    return () => { window.removeEventListener('message', receive); window.removeEventListener('keydown', escape, true); watch.disconnect(); };
  });
  function expand() { expanded = true; dialog.showModal(); void tick().then(() => closeButton?.focus()); }
  function collapsed() { expanded = false; zoom = 1; void tick().then(() => expandButton?.focus({ preventScroll: true })); }
  function close() { dialog.close(); collapsed(); }
</script>

<div class="visual-reply" data-visual-reply={artifact.id} style:--preview-height={`${Math.min(height, 180)}px`}>
  <div class="agent-attachment visual-attachment-header">
    <button class="agent-attachment-open visual-attachment-open" type="button" bind:this={expandButton}
      aria-label={`Expand ${artifact.title}`} title={`Expand ${artifact.title}`} aria-expanded={expanded} onclick={expand}>
      <span class="agent-attachment-type" aria-hidden="true">HTML</span>
      <span class="agent-attachment-name visual-title">{artifact.title}</span>
      <span class="visual-expand-icon"><Icon {PM} name="aspectRatio" /></span>
    </button>
  </div>
  <!-- Promote the existing dialog to the top layer; keep the same live iframe. -->
  <dialog bind:this={dialog} class:modal={expanded} style:--visual-modal-shadow={modalShadow || undefined}
    role={expanded ? 'dialog' : 'group'} aria-label={expanded ? `Expanded ${artifact.title}` : `${artifact.title} preview`}
    onclose={collapsed} oncancel={collapsed}>
    {#if expanded}
      <div class="visual-dialog-toolbar">
        <span class="visual-title">{artifact.title}</span>
        <div class="visual-controls" role="group" aria-label="Visualization controls">
          <button class="iconbtn" type="button" aria-label={`Zoom out ${artifact.title}`} title="Zoom out" disabled={zoom <= .5} onclick={() => zoom = Math.max(.5, zoom - .25)}>−</button>
          <button class="btn ghost visual-zoom" type="button" aria-label={`Reset zoom ${artifact.title}`} title="Reset zoom" onclick={() => zoom = 1}>{Math.round(zoom * 100)}%</button>
          <button class="iconbtn" type="button" aria-label={`Zoom in ${artifact.title}`} title="Zoom in" disabled={zoom >= 2} onclick={() => zoom = Math.min(2, zoom + .25)}><Icon {PM} name="plus" /></button>
          {#if !error}<button class="iconbtn" type="button" aria-label={`Replay ${artifact.title}`} title="Replay" onclick={() => replay++}><Icon {PM} name="rotate" /></button>{/if}
          <button class="iconbtn" type="button" aria-label="Close visualization" title="Close visualization" bind:this={closeButton} onclick={close}><Icon {PM} name="x" /></button>
        </div>
      </div>
    {/if}
    {#if !ready && !error}<p class="visual-loading shimmer-text" role="status">Loading visualization…</p>{/if}
    {#if error}
      <div class="visual-notice"><ErrorNotice error={error} fallbackTitle="Visualization unavailable">
        {#snippet actions()}<button class="btn" type="button" aria-label={`Replay ${artifact.title}`} onclick={() => replay++}>Replay</button>{/snippet}
      </ErrorNotice></div>
    {/if}
    <div class="visual-viewport" class:expanded style:height={expanded ? 'auto' : 'var(--preview-height)'}>
      {#key replay}
        {#if documentHtml}
          <iframe bind:this={iframe} src="/visual-reply.html" title={artifact.title}
            name={`powermove-visual-${artifact.id}`} sandbox="allow-scripts" referrerpolicy="no-referrer"
            allow="camera 'none'; microphone 'none'; geolocation 'none'; clipboard-read 'none'; clipboard-write 'none'; fullscreen 'none'"
            style:width={expanded ? `${100 / zoom}%` : '100%'} style:height={expanded ? `${100 / zoom}%` : `${height}px`} style:transform={`scale(${expanded ? zoom : 1})`}
            onload={initialize}></iframe>
        {/if}
      {/key}
    </div>
  </dialog>
</div>

<style>
  .visual-reply{flex:0 0 min(100%,280px);width:min(100%,280px);min-width:0;min-height:calc(var(--preview-height) + 38px)}
  .visual-attachment-header{margin-bottom:6px}
  .visual-attachment-header .agent-attachment-type{width:28px;white-space:nowrap}
  .visual-attachment-open{width:100%;height:100%;padding-right:3px;border-radius:var(--r-sm)}
  .visual-attachment-open:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
  .visual-title{flex:1;min-width:0;max-width:none;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .visual-expand-icon{display:grid;place-items:center;flex:none;width:20px;height:20px;margin-left:auto;color:var(--tx-3);transition:color var(--dur-1)}
  .visual-expand-icon :global(svg){width:13px;height:13px}
  .visual-attachment-open:hover .visual-expand-icon{color:var(--tx)}
  dialog{display:block;position:static;inset:auto;margin:0;padding:0;width:100%;max-width:none;max-height:none;border:0;background:transparent;color:inherit}
  dialog[open]{display:flex;flex-direction:column;position:fixed;inset:0;margin:auto;width:90vw;height:90vh;max-height:calc(100vh - 32px);background:var(--bg-float);border-radius:var(--r-xl);box-shadow:var(--visual-modal-shadow,var(--shadow-float));overflow:hidden;transform-origin:center}
  dialog::backdrop{background:rgb(var(--ink-rgb) / .32);backdrop-filter:blur(3px)}
  :global(:root[data-theme="dark"]) dialog::backdrop{background:rgb(0 0 0 / .5)}
  .visual-dialog-toolbar{display:flex;align-items:center;flex:none;gap:var(--gap);padding:12px 18px;color:var(--tx);font:var(--fw-semibold) var(--fs-lg)/var(--lh-tight) var(--f-ui)}
  .visual-controls{display:flex;align-items:center;flex:none;gap:2px}
  .visual-controls :global(.pm-icon){width:14px;height:14px}
  .visual-zoom{min-width:48px;padding:0 6px;font-variant-numeric:tabular-nums}
  .visual-controls .iconbtn:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
  .visual-loading{margin:6px 0;color:var(--tx-3);font:var(--fs-sm)/var(--lh) var(--f-ui)}
  .visual-notice{min-width:0;margin-bottom:8px}
  dialog[open] .visual-notice{flex:none;max-height:40%;overflow:auto;margin:0 18px 12px}
  dialog[open] .visual-loading{margin:0 18px 12px}
  .visual-viewport{position:relative;width:100%;overflow:auto;border-radius:var(--r-md);background:var(--bg-panel);contain:layout paint;overscroll-behavior:contain}
  .visual-viewport.expanded{flex:1;min-height:0;width:auto;margin:0 18px 18px}
  iframe{display:block;border:0;transform-origin:top left}
  @media(prefers-reduced-motion:reduce){.visual-expand-icon{transition:none}}
</style>
