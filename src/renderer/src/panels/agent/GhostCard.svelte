<script lang="ts">
  import { mountGhostEdgeField } from './ui-placement-ghost-field';
  import { ghostRowCount, type GhostRect } from './ui-placement-geometry';
  import type { UIPlacement } from './ui-placement';

  /* Without a rect the card is a real slot in the dock, holding the space the
     finished panel will take. With one it is pinned over the panel being
     rebuilt, which keeps its own place in the layout. */
  let { placement, rect = null }: { placement: UIPlacement; rect?: GhostRect | null } = $props();

  /* A preview of the shape that is coming: a label and a control on each row,
     at the uneven widths real controls have. Seven of them, so the pattern does
     not read as a repeat once it fills a tall panel. */
  const pattern = [
    { label: 44, track: 56, fill: 68 },
    { label: 31, track: 78, fill: 34 },
    { label: 52, track: 44, fill: 55 },
    { label: 37, track: 66, fill: 22 },
    { label: 47, track: 71, fill: 81 },
    { label: 28, track: 49, fill: 41 },
    { label: 55, track: 62, fill: 12 }
  ];

  let body = $state<HTMLElement | null>(null);
  let rowCount = $state(0);
  const rows = $derived(Array.from({ length: rowCount }, (_, index) => pattern[index % pattern.length]!));

  const ghostEdgeField = (host: HTMLElement) => ({ destroy: mountGhostEdgeField(host) });

  /* The ghost is given its height by the dock or by the panel it covers, so the
     rows are counted from the space that arrives rather than guessed. */
  $effect(() => {
    const element = body;
    if (!element) return;
    const observer = new ResizeObserver(entries => {
      const box = entries[0]?.contentRect;
      if (box) rowCount = ghostRowCount(box.height);
    });
    observer.observe(element);
    return () => observer.disconnect();
  });
</script>

<div
  class="ui-placement-ghost"
  class:pinned={!!rect}
  class:new-panel={placement.kind === 'dock'}
  data-ui-placement-ghost={placement.id}
  role="status"
  aria-live="polite"
  aria-atomic="true"
  style:left={rect ? `${rect.left}px` : null}
  style:top={rect ? `${rect.top}px` : null}
  style:width={rect ? `${rect.width}px` : null}
  style:height={rect ? `${rect.height}px` : null}
>
  <div use:ghostEdgeField class="ghost-edge-field" aria-hidden="true"></div>
  <header class="ghost-head">
    <i aria-hidden="true"></i>
    <span>{placement.kind === 'dock' ? 'Building' : 'Updating'} {placement.label}…</span>
  </header>
  <div class="ghost-rows" bind:this={body} aria-hidden="true">
    {#each rows as row, index}
      <div class="ghost-row" style:animation-delay={`${(index % 5) * 0.16}s`}>
        <i style:width={`${row.label}px`}></i>
        <b style:width={`${row.track}%`} style:--fill={`${row.fill}%`}></b>
      </div>
    {/each}
  </div>
</div>

<style>
  /* The generation field is blended, so the ghost isolates itself: the light
     pools inside the placeholder instead of bleeding over live panels. */
  .ui-placement-ghost {
    position: relative;
    box-sizing: border-box;
    /* A reserved slot is sized like a panel and yields before the panels do. */
    flex: 0 1 172px;
    min-height: 78px;
    pointer-events: none;
    user-select: none;
    overflow: hidden;
    isolation: isolate;
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    border-radius: var(--r-lg, 12px);
    background: var(--bg-panel, #fff);
    box-shadow: var(--shadow-panel);
    color: var(--tx, #1b1d23);
    animation: ghost-reserve var(--dur-3, 160ms) var(--ease, ease-out);
  }
  /* A panel that already exists keeps its own frame; the ghost sits inside it. */
  .pinned {
    position: fixed;
    z-index: 35;
    flex: none;
    border: 1px solid var(--line-2);
    border-radius: var(--r-md, 8px);
    box-shadow: none;
    animation: none;
  }
  .new-panel {
    border: 1px dashed color-mix(in srgb, var(--tx, #1b1d23) 13%, transparent);
    background: var(--bg-panel-2, #f5f5f7);
    box-shadow: none;
  }
  /* The field is light, so it lifts a dark panel and tints a light one. */
  .ghost-edge-field {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    overflow: hidden;
    pointer-events: none;
    mix-blend-mode: multiply;
  }
  :global(:root[data-theme='dark']) .ghost-edge-field { mix-blend-mode: screen; }
  /* The renderer state is written by the mount, so the selector stays global. */
  :global(.ghost-edge-field[data-ghost-renderer='static']) { opacity: 0; }
  /* The head reads as the panel header it stands in for. */
  .ghost-head {
    position: relative;
    z-index: 1;
    flex: none;
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    height: var(--hdr-h, 38px);
    padding: 0 12px 0 14px;
    color: var(--tx-2);
    font: var(--fw-medium, 500) var(--fs-md, 13px) var(--f-ui, sans-serif);
    letter-spacing: -0.005em;
  }
  .ghost-head span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ghost-head i {
    flex: 0 0 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    box-shadow: 0 0 8px color-mix(in srgb, var(--accent, #ff6b1a) 55%, transparent);
    animation: ghost-breathe 3.6s ease-in-out infinite;
  }
  .ghost-rows {
    position: relative;
    z-index: 1;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 11px;
    padding: 5px 14px 14px;
    overflow: hidden;
    /* The rows are a preview, not a list: the last of them settles out instead
       of stopping on a hard edge. */
    mask: linear-gradient(to bottom, #000 calc(100% - 26px), transparent) no-repeat;
  }
  .ghost-row { display: flex; align-items: center; gap: 10px; animation: ghost-line 3.6s ease-in-out infinite; }
  .ghost-row i { flex: 0 1 auto; height: 6px; border-radius: 3px; background: color-mix(in srgb, currentColor 17%, transparent); }
  .ghost-row b {
    flex: 0 1 auto;
    min-width: 0;
    height: 6px;
    border-radius: 3px;
    background: linear-gradient(
      90deg,
      color-mix(in srgb, currentColor 17%, transparent) 0 var(--fill, 50%),
      color-mix(in srgb, currentColor 7%, transparent) var(--fill, 50%) 100%
    );
  }
  /* A narrow ghost drops the label column rather than crowd it. */
  @container (max-width: 168px) { .ghost-row i { display: none; } }
  /* One shared rhythm with the shader: a quicker inhale, a longer exhale. */
  @keyframes ghost-breathe {
    0%, 100% { opacity: 0.4; transform: scale(0.86); }
    38% { opacity: 1; transform: scale(1.16); }
  }
  @keyframes ghost-line {
    0%, 100% { opacity: 0.55; }
    38% { opacity: 1; }
  }
  /* The dock opens the slot instead of snapping the panels aside. */
  @keyframes ghost-reserve {
    from { flex-basis: 0; opacity: 0; margin-bottom: calc(var(--gap, 10px) * -1); }
  }
  @media (prefers-reduced-motion: reduce) {
    .ui-placement-ghost { animation: none; }
    .ghost-edge-field { display: none; }
    .ghost-head i, .ghost-row { animation: none; }
  }
</style>
