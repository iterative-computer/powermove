<script lang="ts">
  import { durationLabel, type ToolDetail, type ToolsRow } from './activity-rows';
  import { toolGlyph, type ToolGlyph } from './tool-icons';

  /* A tool group, harness-style: one quiet header row ("Ran commands and
     edited files · 3 calls · 4s") that discloses compact call rows. Each call
     row is icon + label + a mono chip with its argument; rows that carry a
     result excerpt expand in place. Auto-open while running, settles closed. */

  let { row, animated = false }: { row: ToolsRow & { pulsing?: boolean }; animated?: boolean } = $props();

  const isRunning = $derived(row.status === 'running');
  const summary = $derived(isRunning ? (row.currentLabel || row.label) : row.label);
  const span = $derived(row.startedAt !== undefined && row.endedAt !== undefined ? durationLabel(row.endedAt - row.startedAt) : '');
  const meta = $derived(isRunning ? '' : [
    row.toolCount > 1 ? `${row.toolCount} calls` : '',
    span
  ].filter(Boolean).join(' · '));

  let openRows = $state(new Set<string>());
  const rowKey = (detail: ToolDetail, index: number): string => `${detail.id}-${index}`;
  const expandable = (detail: ToolDetail): boolean => Boolean(detail.output);
  function toggleRow(key: string): void {
    const next = new Set(openRows);
    if (next.has(key)) next.delete(key); else next.add(key);
    openRows = next;
  }

  const statusLabel = (status: ToolDetail['status']): string => {
    if (status === 'running') return 'Running';
    if (status === 'error') return 'Failed';
    if (status === 'continued') return 'Continued';
    return 'Done';
  };
  const callSpan = (detail: ToolDetail): string =>
    detail.startedAt !== undefined && detail.endedAt !== undefined ? durationLabel(detail.endedAt - detail.startedAt) : '';
</script>

{#snippet call(detail: ToolDetail, glyph: ToolGlyph, canOpen: boolean)}
  <svg class="agent-tool-glyph" viewBox="0 0 24 24" aria-hidden="true" data-family={detail.family}>
    <path d={glyph.d} fill={glyph.fill ? 'currentColor' : 'none'} stroke={glyph.fill ? 'none' : 'currentColor'} />
  </svg>
  {#if canOpen}
    <svg class="agent-tool-row-chevron" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 9l6 6 6-6" /></svg>
  {/if}
  <span>{detail.label}</span>
  {#if detail.detail}<code class="agent-tool-chip">{detail.detail}</code>{/if}
  {#if detail.status === 'running'}
    <i class="agent-tool-spinner" aria-hidden="true"></i>
    <em class="panel-sr-only">Running</em>
  {:else if detail.status === 'error' || detail.status === 'continued'}
    <em>{statusLabel(detail.status)}</em>
  {:else if callSpan(detail)}
    <em class="is-time">{callSpan(detail)}</em>
  {/if}
{/snippet}

<details
  class="agent-trace-tool agent-tool-activity"
  class:is-error={row.status === 'error'}
  class:is-partial={row.status === 'partial'}
  class:is-running={isRunning}
  class:is-pulsing={animated && row.pulsing}
  open={isRunning}
  title={row.detail?.length ? row.detail.join('\n') : undefined}
>
  <summary class:has-status={isRunning || row.status === 'error' || row.status === 'partial'}>
    <svg class="agent-tool-chevron" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 9l6 6 6-6" /></svg>
    <span>{summary}{#if isRunning && row.currentDetail}<code class="agent-tool-chip">{row.currentDetail}</code>{/if}</span>
    {#if isRunning}
      <em>Working</em>
    {:else if row.status === 'error'}
      <em>Failed</em>
    {:else if row.status === 'partial'}
      <em>{row.successCount} completed · {row.failedCount} failed</em>
    {:else if meta}
      <small>{meta}</small>
    {/if}
  </summary>

  {#if row.details.length}
    <div class="agent-tool-details" aria-label="Tool activity details">
      {#each row.details as detail, index (rowKey(detail, index))}
        {@const key = rowKey(detail, index)}
        {@const glyph = toolGlyph(detail.family)}
        {@const open = openRows.has(key)}
        {@const canOpen = expandable(detail)}
        <div class:is-failed={detail.status === 'error'} class:is-running={detail.status === 'running'} class:is-open={open}>
          {#if canOpen}
            <button class="agent-tool-row" type="button" aria-expanded={open} onclick={() => toggleRow(key)}>
              {@render call(detail, glyph, true)}
            </button>
          {:else}
            <div class="agent-tool-row">
              {@render call(detail, glyph, false)}
            </div>
          {/if}
          {#if canOpen}
            <div class="agent-tool-output" class:is-open={open}>
              <pre>{detail.output}</pre>
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</details>

<style>
  .agent-tool-activity {
    position: relative;
    display: block;
    min-width: 0;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--tx-2);
    interpolate-size: allow-keywords;
  }
  /* Disclosure body slides open like the harness grid-rows trick; native
     details keeps the semantics and the click target free. */
  .agent-tool-activity::details-content {
    display: block;
    block-size: 0;
    overflow: clip;
    opacity: 0;
    transition: block-size 300ms cubic-bezier(.23, 1, .32, 1), opacity 240ms ease-out, content-visibility 300ms allow-discrete;
  }
  .agent-tool-activity[open]::details-content { block-size: auto; opacity: 1; }

  summary {
    display: grid;
    grid-template-columns: 14px minmax(0, 1fr);
    grid-template-rows: auto;
    align-items: start;
    gap: 2px 6px;
    min-width: 0;
    margin: 0 -6px;
    padding: 5px 6px;
    border-radius: var(--r-sm);
    cursor: pointer;
    list-style: none;
    font-size: 12.5px;
    line-height: 1.45;
    user-select: text;
    -webkit-user-select: text;
    transition: background var(--dur-2), color var(--dur-2);
  }
  summary:hover { background: var(--ink-1); color: var(--tx); }
  summary:focus-visible { outline: 1px solid var(--accent); outline-offset: -2px; }
  summary::-webkit-details-marker { display: none; }
  summary.has-status { grid-template-rows: auto auto; }
  summary > span { grid-column: 2; grid-row: 1; min-width: 0; overflow-wrap: anywhere; }
  summary > em, summary > small { grid-column: 2; grid-row: 2; text-align: left; line-height: 1.4; font-size: 11px; font-style: normal; color: var(--tx-3); }
  summary > small { font-variant-numeric: tabular-nums; }
  summary > span > .agent-tool-chip { margin-left: 6px; }
  .agent-tool-chevron {
    grid-column: 1; grid-row: 1; align-self: center; width: 12px; height: 12px; margin-top: 1px;
    fill: none; stroke: var(--tx-4); stroke-width: 2.2; stroke-linecap: round; stroke-linejoin: round;
    transform: rotate(-90deg); transition: transform 200ms var(--ease);
  }
  details[open] > summary .agent-tool-chevron { transform: rotate(0deg); }
  /* The live group shimmers its header text instead of pulsing a card. */
  .is-running > summary > span {
    background: linear-gradient(100deg, var(--tx-3) 30%, var(--tx) 50%, var(--tx-3) 70%);
    background-size: 200% 100%;
    -webkit-background-clip: text; background-clip: text; color: transparent;
    animation: shimmer-sweep 1.6s linear infinite;
  }
  .is-running > summary > span > .agent-tool-chip { -webkit-background-clip: border-box; background-clip: border-box; color: var(--tx-2); }
  .is-error > summary > span { color: var(--danger); }
  .is-error > summary > em { color: var(--danger); }

  .agent-tool-details {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 1px;
    margin: 2px 0 2px 5px;
    padding: 2px 0 2px 10px;
    border-left: 1px solid var(--line-2);
    font-size: 12px;
    color: var(--tx-2);
    user-select: text;
    -webkit-user-select: text;
  }
  .agent-tool-details > div { min-width: 0; animation: agent-fade-up 300ms cubic-bezier(.23, 1, .32, 1) both; }
  .agent-tool-row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    min-width: 0;
    min-height: 26px;
    margin: 0;
    padding: 2px 6px 2px 4px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    transition: background var(--dur-2);
  }
  button.agent-tool-row { cursor: pointer; }
  button.agent-tool-row:hover, .is-open > .agent-tool-row { background: var(--ink-1); }
  button.agent-tool-row:focus-visible { outline: 1px solid var(--accent); outline-offset: -1px; }
  .agent-tool-row > span { min-width: 0; flex: none; max-width: 100%; overflow-wrap: anywhere; font-weight: var(--fw-medium); color: var(--tx); }
  .agent-tool-row > em { flex: none; margin-left: auto; font-size: 11px; font-style: normal; color: var(--tx-3); font-variant-numeric: tabular-nums; }
  .agent-tool-glyph { width: 13px; height: 13px; flex: none; color: var(--tx-3); stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; transition: opacity var(--dur-2); }
  .agent-tool-row-chevron {
    position: absolute; left: 4px; top: 50%; width: 12px; height: 12px; margin-top: -6px;
    fill: none; stroke: var(--tx-3); stroke-width: 2.2; stroke-linecap: round; stroke-linejoin: round;
    opacity: 0; transform: rotate(-90deg); transition: opacity var(--dur-2), transform 200ms var(--ease);
  }
  /* Hover swaps the glyph for the chevron, as the harness does. */
  button.agent-tool-row:hover > .agent-tool-glyph, .is-open > .agent-tool-row > .agent-tool-glyph { opacity: 0; }
  button.agent-tool-row:hover > .agent-tool-row-chevron, .is-open > .agent-tool-row > .agent-tool-row-chevron { opacity: 1; }
  .is-open > .agent-tool-row > .agent-tool-row-chevron { transform: rotate(0deg); }
  .agent-tool-chip {
    display: inline-block;
    min-width: 0;
    max-width: 100%;
    flex: 0 1 auto;
    padding: 0 5px;
    border-radius: var(--r-xs);
    background: var(--bg-field);
    color: var(--tx-2);
    font: 11px/1.7 var(--f-mono);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    vertical-align: baseline;
  }
  .agent-tool-spinner {
    width: 11px; height: 11px; flex: none; margin-left: auto; border-radius: 50%;
    border: 1.5px solid var(--line-2); border-top-color: var(--tx-2);
    animation: agent-spin .7s linear infinite;
  }
  .is-failed > .agent-tool-row > span, .is-failed > .agent-tool-row > em, .is-failed > .agent-tool-row > .agent-tool-glyph { color: var(--danger); }
  .agent-tool-output {
    display: grid;
    grid-template-rows: 0fr;
    opacity: 0;
    transition: grid-template-rows 300ms cubic-bezier(.23, 1, .32, 1), opacity 240ms ease-out;
  }
  .agent-tool-output.is-open { grid-template-rows: 1fr; opacity: 1; }
  .agent-tool-output > pre {
    min-height: 0;
    overflow: hidden;
    margin: 0 0 0 12px;
    padding: 0 0 0 10px;
    border-left: 1px solid var(--line);
    color: var(--tx-2);
    font: 11px/1.6 var(--f-mono);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .agent-tool-output.is-open > pre { margin-top: 2px; margin-bottom: 6px; }

  @media (prefers-reduced-motion: reduce) {
    .agent-tool-activity::details-content, .agent-tool-chevron, .agent-tool-row-chevron, .agent-tool-output { transition: none; }
    .agent-tool-details > div { animation: none; }
    .is-running > summary > span { animation: none; background: none; -webkit-background-clip: border-box; background-clip: border-box; color: var(--tx-2); }
    .agent-tool-spinner { animation: none; border-top-color: var(--line-2); }
  }
</style>
