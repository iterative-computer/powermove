<script lang="ts">
  import { onMount } from 'svelte';
  import type { PMRegistry } from '../legacy/registry';
  import { GHOST_DELAY_MS, holdBody, measureGhostTiles, readProjectLoading, type GhostKind, type GhostTile } from './project-ghost';

  let { PM }: { PM: PMRegistry } = $props();

  /** The ghost is in the DOM: from the start of a load until its fade-out ends. */
  let shown = $state(false);
  /** Measured panels; empty means the default layout. */
  let tiles = $state.raw<GhostTile[]>([]);
  let frame = $state.raw({ top: 0, left: 0, width: 0, height: 0 });
  let message = $state('');
  let ghost = $state<HTMLDivElement>();

  let release: (() => void) | null = null;
  let observer: ResizeObserver | null = null;
  let exit: Animation | null = null;
  let startedAt = 0;

  const generic: Array<{ area: string; kind: GhostKind; rows: number }> = [
    { area: 'left', kind: 'list', rows: 9 },
    { area: 'center', kind: 'viewer', rows: 0 },
    { area: 'bottom', kind: 'timeline', rows: 6 },
    { area: 'right', kind: 'list', rows: 12 }
  ];

  function reducedMotion(): boolean {
    return !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
  }

  function measure(body: HTMLElement): void {
    const rect = body.getBoundingClientRect();
    frame = { top: rect.top, left: rect.left, width: rect.width, height: rect.height };
    const next = measureGhostTiles(body);
    // Panels can be torn down mid-load; keep the last shape rather than jump to the default.
    if (next.length || !tiles.length) tiles = next;
  }

  function begin(name: string): void {
    message = `Opening ${name || 'Untitled'}`;
    const body = document.getElementById('body');
    if (!body) return;
    release ??= holdBody(body);
    if (shown && !exit) return;
    exit?.cancel();
    exit = null;
    startedAt = performance.now();
    tiles = [];
    measure(body);
    shown = true;
    if (!observer && typeof ResizeObserver !== 'undefined') {
      observer = new ResizeObserver(() => measure(body));
      observer.observe(body);
    }
  }

  function end(): void {
    message = '';
    release?.();
    release = null;
    observer?.disconnect();
    observer = null;
    if (!shown || exit) return;
    const node = ghost;
    // A load that beat the delay never showed anything; with reduced motion there is no fade.
    if (!node || reducedMotion() || performance.now() - startedAt < GHOST_DELAY_MS || typeof node.animate !== 'function') {
      shown = false;
      return;
    }
    exit = node.animate(
      [{ opacity: getComputedStyle(node).opacity }, { opacity: 0 }],
      { duration: 120, easing: 'cubic-bezier(.45,0,.2,1)', fill: 'forwards' }
    );
    const current = exit;
    current.onfinish = () => {
      if (exit !== current) return;
      exit = null;
      shown = false;
    };
  }

  function sync(): void {
    const loading = readProjectLoading(PM);
    if (loading) begin(loading.name);
    else end();
  }

  onMount(() => {
    // The static HTML only owns the pre-JavaScript frame. Once the chrome is
    // mounted, this component owns loading visuals and their cleanup.
    document.getElementById('boot-ghost')?.remove();
    const off = PM.bus?.on?.('project:loading', sync);
    sync();
    return () => {
      off?.();
      exit?.cancel();
      exit = null;
      shown = false;
      end();
    };
  });
</script>

{#snippet content(kind: GhostKind, rows: number)}
  <span class="pg-head"></span>
  {#if kind === 'viewer'}
    <span class="pg-stage"></span>
  {:else}
    <span class="pg-rows">
      {#each { length: rows } as _}<span class="pg-row"></span>{/each}
    </span>
  {/if}
{/snippet}

<div class="project-loading-status" role="status" aria-live="polite">{message}</div>

{#if shown}
  <div
    class="project-ghost"
    class:generic={!tiles.length}
    style:top="{frame.top}px"
    style:left="{frame.left}px"
    style:width="{frame.width}px"
    style:height="{frame.height}px"
    aria-hidden="true"
    data-project-ghost
    bind:this={ghost}
  >
    {#if tiles.length}
      {#each tiles as tile}
        <div
          class="pg-tile pg-{tile.kind}"
          style:left="{tile.x}px"
          style:top="{tile.y}px"
          style:width="{tile.width}px"
          style:height="{tile.height}px"
        >{@render content(tile.kind, tile.rows)}</div>
      {/each}
    {:else}
      {#each generic as tile}
        <div class="pg-tile pg-{tile.kind}" style:grid-area={tile.area}>{@render content(tile.kind, tile.rows)}</div>
      {/each}
    {/if}
  </div>
{/if}
