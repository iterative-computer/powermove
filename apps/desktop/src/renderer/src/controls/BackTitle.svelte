<script lang="ts">
  /* A stable page title that doubles as Back; text changes are immediate by default. */
  import Scritto from '@scritto/svelte';
  import Icon from '../panels/Icon.svelte';

  let {
    PM,
    text,
    back = false,
    animateText = false,
    label,
    onback,
    class: className = ''
  }: {
    PM: Record<string, any>;
    text: string;
    back?: boolean;
    animateText?: boolean;
    /** Accessible name while it is a button; defaults to "Back to {text}". */
    label?: string;
    onback?: () => void;
    class?: string;
  } = $props();

  function go(): void {
    if (back) onback?.();
  }
</script>

<h1 class="pm-back-title {className}" class:is-back={back}>
  <!-- Focusable only while it is role=button (back). -->
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_no_noninteractive_tabindex -->
  <span
    class="pm-back-title-inner"
    role={back ? 'button' : undefined}
    tabindex={back ? 0 : undefined}
    aria-label={back ? label ?? `Back to ${text}` : undefined}
    onclick={go}
    onkeydown={(event) => { if (back && (event.key === 'Enter' || event.key === ' ')) { event.preventDefault(); go(); } }}
  >
    <span class="pm-back-title-chev" aria-hidden="true"><Icon {PM} name="chev" /></span>
    {#if animateText}
      <Scritto value={text} />
    {:else}
      <span>{text}</span>
    {/if}
  </span>
</h1>

<style>
  .pm-back-title { margin: 0; font-size: 22px; line-height: 28px; font-weight: var(--fw-medium); letter-spacing: -.015em; color: var(--tx); }
  .pm-back-title-inner { display: inline-flex; align-items: center; vertical-align: top; border-radius: var(--r-sm); transition: color var(--dur-1) var(--ease); }
  .pm-back-title-chev { display: inline-flex; align-items: center; width: 0; height: 28px; overflow: hidden; opacity: 0; color: var(--tx-3); transition: width 100ms var(--ease), opacity 100ms var(--ease), color var(--dur-1) var(--ease); }
  .pm-back-title-chev :global(svg) { flex: none; width: 17px; height: 17px; transform: rotate(180deg); }
  .is-back .pm-back-title-chev { width: 22px; opacity: 1; }
  .is-back .pm-back-title-inner { cursor: default; user-select: none; -webkit-user-select: none; }
  .is-back .pm-back-title-inner:hover, .is-back .pm-back-title-inner:hover .pm-back-title-chev { color: var(--tx-2); }
  .is-back .pm-back-title-inner:focus-visible { outline: 0; box-shadow: 0 0 0 3px var(--ink-2); }
  @media (prefers-reduced-motion: reduce) { .pm-back-title-chev { transition: none; } }
</style>
