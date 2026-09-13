<script lang="ts">
  import type { ToolDetail, ToolsRow } from './activity-rows';
  import { durationLabel } from './activity-rows';
  import { toolGlyph, type ToolGlyph } from './tool-icons';
  import { revealText } from './text-reveal';

  /* One stretch of agent work, in the beautiful-ui ToolChips grammar: a quiet
     header ("4 tool calls · 1 failed · 21s"), then flat rows
     of glyph + label + a hairline chip holding the argument. Reasoning is a
     row like any other ("Thinking" + its opening sentence). Rows with more to
     show expand under a hairline; the chevron takes the glyph's place on
     hover. Edited files close the group as mono chips. */

  let { row, animated = false, live = false }: {
    row: ToolsRow & { pulsing?: boolean };
    animated?: boolean;
    /** This group is the run's newest activity and the run is still going. */
    live?: boolean;
  } = $props();

  /* Disclosure follows the RUN, not the instant: while this group is the
     newest thing in a live run it stays open even between calls (a call
     ending is not the agent moving on). It settles closed once prose lands
     after it or the run ends. A manual toggle wins for the group's lifetime. */
  let manualOpen = $state<boolean | null>(null);
  const open = $derived(manualOpen ?? live);
  function onToggle(event: Event): void {
    const element = event.currentTarget as HTMLDetailsElement;
    if (element.open !== open) manualOpen = element.open;
  }
  const isRunning = $derived(row.status === 'running');
  /* Reasoning alone needs no header: the thought row is its own disclosure. */
  const headerless = $derived(row.toolCount === 0);
  const span = $derived(row.startedAt !== undefined && row.endedAt !== undefined ? durationLabel(row.endedAt - row.startedAt) : '');
  const meta = $derived(live || isRunning ? '' : [
    row.status === 'partial' ? `${row.failedCount} failed` : '',
    span
  ].filter(Boolean).join(' · '));

  /* Manual toggles win; otherwise a live thought is open so its reasoning is
     readable as it streams, and everything else is closed. */
  let manual = $state<Record<string, boolean>>({});
  const rowKey = (detail: ToolDetail, index: number): string => `${detail.id}-${index}`;
  const expandable = (detail: ToolDetail): boolean => Boolean(detail.output);
  const isOpen = (detail: ToolDetail, key: string): boolean =>
    key in manual ? manual[key]! : detail.kind === 'thought' && detail.status === 'running';
  function toggleRow(detail: ToolDetail, key: string): void {
    manual = { ...manual, [key]: !isOpen(detail, key) };
  }
  const callSpan = (detail: ToolDetail): string =>
    detail.kind === 'tool' && detail.startedAt !== undefined && detail.endedAt !== undefined
      ? durationLabel(detail.endedAt - detail.startedAt) : '';
</script>

{#snippet call(detail: ToolDetail, glyph: ToolGlyph, canOpen: boolean, open: boolean)}
  {#if detail.status === 'running' && detail.kind === 'tool'}
    <i class="agent-tool-spinner" aria-hidden="true"></i>
  {:else}
    <svg class="agent-tool-glyph" viewBox="0 0 24 24" aria-hidden="true" data-family={detail.family}>
      <path d={glyph.d} fill={glyph.fill ? 'currentColor' : 'none'} stroke={glyph.fill ? 'none' : 'currentColor'} />
    </svg>
  {/if}
  {#if canOpen}
    <svg class="agent-tool-row-chevron" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 9l6 6 6-6" /></svg>
  {/if}
  <span class:shimmer-text={headerless && detail.kind === 'thought' && detail.status === 'running' && animated}>{detail.label}</span>
  {#if detail.detail && !(open && detail.kind === 'thought')}
    <code class="agent-tool-chip" class:is-prose={detail.kind === 'thought'}>{detail.detail}</code>
  {/if}
  {#if detail.status === 'error'}
    <em>Failed</em>
  {:else if detail.status === 'continued'}
    <em>Continued</em>
  {:else if callSpan(detail)}
    <em>{callSpan(detail)}</em>
  {/if}
{/snippet}

{#snippet rows()}
  <div class="agent-tool-details" aria-label="Tool activity details">
    {#each row.details as detail, index (rowKey(detail, index))}
      {@const key = rowKey(detail, index)}
      {@const glyph = toolGlyph(detail.family)}
      {@const canOpen = expandable(detail)}
      {@const open = canOpen && isOpen(detail, key)}
      <div
        class:is-failed={detail.status === 'error'}
        class:is-running={detail.status === 'running'}
        class:is-open={open}
        class:is-thought={detail.kind === 'thought'}
      >
        {#if canOpen}
          <button class="agent-tool-row" type="button" aria-expanded={open} onclick={() => toggleRow(detail, key)}>
            {@render call(detail, glyph, true, open)}
          </button>
        {:else}
          <div class="agent-tool-row">
            {@render call(detail, glyph, false, false)}
          </div>
        {/if}
        {#if canOpen}
          <div class="agent-tool-output" class:is-open={open}>
            <div>
              {#if detail.kind === 'thought'}
                <p class="agent-tool-thought">{#each String(detail.output).split(/(\s+)/) as word, wi (wi)}<span use:revealText={animated && detail.status === 'running' && Boolean(word.trim())}>{word}</span>{/each}</p>
              {:else}
                <pre>{detail.output}</pre>
              {/if}
            </div>
          </div>
        {/if}
      </div>
    {/each}
  </div>
  {#if row.files.length && !live && !isRunning}
    <div class="agent-tool-files">
      {#each row.files as file, index (file)}
        <code class="agent-tool-file" style="--i:{index}">{file}</code>
      {/each}
    </div>
  {/if}
{/snippet}

{#if headerless}
  <div class="agent-trace-tool agent-tool-activity is-headerless" class:is-pulsing={animated && row.pulsing}>
    {#if row.details.length}{@render rows()}{/if}
  </div>
{:else}
  <details
    class="agent-trace-tool agent-tool-activity"
    class:is-error={row.status === 'error'}
    class:is-partial={row.status === 'partial'}
    class:is-running={isRunning}
    class:is-pulsing={animated && row.pulsing}
    {open}
    ontoggle={onToggle}
    title={row.detail?.length ? row.detail.join('\n') : undefined}
  >
    <summary>
      <svg class="agent-tool-chevron" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 9l6 6 6-6" /></svg>
      <span class:shimmer-text={(live || isRunning) && animated}>{row.label}{#if row.status === 'error'} <em>Failed</em>{:else if meta} <em>{meta}</em>{/if}</span>
    </summary>
    {#if row.details.length}{@render rows()}{/if}
  </details>
{/if}

<style>
  /* Scale: header and labels at --fs-sm, chips and meta at --fs-xs, all on
     the panel's UI face. One row is 26px; the harness is 28 on a 14px base. */
  .agent-tool-activity {
    display: block;
    min-width: 0;
    border: 0;
    color: var(--tx-2);
    font-size: var(--fs-sm);
    interpolate-size: allow-keywords;
  }
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
    grid-template-columns: 12px minmax(0, 1fr);
    align-items: center;
    column-gap: 6px;
    width: fit-content;
    max-width: 100%;
    min-width: 0;
    padding: 3px 6px 3px 4px;
    border-radius: var(--r-sm);
    list-style: none;
    cursor: pointer;
    line-height: 1.5;
    color: var(--tx-2);
    transition: background var(--dur-2), color var(--dur-2);
  }
  summary:hover { background: var(--ink-1); color: var(--tx); }
  summary:focus-visible { outline: 1px solid var(--accent); outline-offset: -2px; }
  summary::-webkit-details-marker { display: none; }
  summary > span { min-width: 0; overflow-wrap: anywhere; }
  summary em { margin-left: 6px; font-style: normal; font-size: var(--fs-xs); color: var(--tx-3); font-variant-numeric: tabular-nums; }
  .is-headerless { padding-top: 1px; }
  .agent-tool-chevron {
    width: 12px; height: 12px;
    fill: none; stroke: currentColor; stroke-width: 2.2; stroke-linecap: round; stroke-linejoin: round;
    transform: rotate(-90deg); transition: transform 200ms var(--ease);
  }
  details[open] > summary .agent-tool-chevron { transform: rotate(0deg); }
  .is-error > summary > span, .is-error > summary em { color: var(--danger); }

  .agent-tool-details {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    padding: 2px 0 2px;
    color: var(--tx-2);
    user-select: text;
    -webkit-user-select: text;
  }
  .agent-tool-details > div { min-width: 0; animation: agent-fade-up 300ms cubic-bezier(.23, 1, .32, 1) both; }
  .agent-tool-row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
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
  .agent-tool-row > span { min-width: 0; flex: 0 1 auto; overflow-wrap: anywhere; font-weight: var(--fw-medium); color: var(--tx); }
  /* The label keeps its width and the chip takes the rest, truncating; only a
     label with nothing beside it may wrap. */
  .agent-tool-row > span:has(+ .agent-tool-chip) { flex-shrink: 0; overflow-wrap: normal; }
  .agent-tool-row > em { flex: none; margin-left: auto; font-style: normal; font-size: var(--fs-xs); color: var(--tx-3); font-variant-numeric: tabular-nums; }
  .is-thought > .agent-tool-row > span { color: var(--tx-2); }
  .agent-tool-glyph, .agent-tool-spinner { width: 13px; height: 13px; flex: none; }
  .agent-tool-glyph { color: var(--tx-3); stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; transition: opacity var(--dur-2); }
  .agent-tool-spinner { border-radius: 50%; border: 1.5px solid var(--line-2); border-top-color: var(--tx-2); animation: agent-spin .7s linear infinite; }
  .agent-tool-row-chevron {
    position: absolute; left: 4px; top: 50%; width: 12px; height: 12px; margin-top: -6px;
    fill: none; stroke: var(--tx-3); stroke-width: 2.2; stroke-linecap: round; stroke-linejoin: round;
    opacity: 0; transform: rotate(-90deg); transition: opacity var(--dur-2), transform 200ms var(--ease);
  }
  button.agent-tool-row:hover > :is(.agent-tool-glyph, .agent-tool-spinner), .is-open > .agent-tool-row > :is(.agent-tool-glyph, .agent-tool-spinner) { opacity: 0; }
  button.agent-tool-row:hover > .agent-tool-row-chevron, .is-open > .agent-tool-row > .agent-tool-row-chevron { opacity: 1; }
  .is-open > .agent-tool-row > .agent-tool-row-chevron { transform: rotate(0deg); }

  /* The chip: a field-toned pill with a hairline ring, roomy enough to read. */
  .agent-tool-chip {
    display: inline-block;
    min-width: 0;
    max-width: 100%;
    flex: 1 1 0%;
    height: 20px;
    padding: 0 6px;
    border-radius: var(--r-sm);
    background: var(--ink-1);
    box-shadow: inset 0 0 0 var(--hairline) var(--line-2);
    color: var(--tx-2);
    font: var(--fs-xs)/20px var(--f-mono);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .agent-tool-chip.is-prose { font-family: var(--f-ui); font-size: var(--fs-xs); color: var(--tx-3); }
  .is-failed > .agent-tool-row > span, .is-failed > .agent-tool-row > em, .is-failed > .agent-tool-row > .agent-tool-glyph { color: var(--danger); }

  .agent-tool-output {
    display: grid;
    grid-template-rows: 0fr;
    opacity: 0;
    transition: grid-template-rows 300ms cubic-bezier(.23, 1, .32, 1), opacity 240ms ease-out;
  }
  .agent-tool-output.is-open { grid-template-rows: 1fr; opacity: 1; }
  .agent-tool-output > div { min-height: 0; overflow: hidden; }
  .agent-tool-output pre, .agent-tool-thought {
    margin: 1px 0 4px 10px;
    padding: 2px 0 2px 14px;
    border-left: 1px solid var(--line-2);
    color: var(--tx-2);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .agent-tool-output pre { font: var(--fs-xs)/1.6 var(--f-mono); }
  .agent-tool-thought { font-size: var(--fs-sm); line-height: 1.55; color: var(--tx-3); }

  .agent-tool-files { display: flex; flex-wrap: wrap; gap: 5px; margin: 6px 0 2px 4px; padding-top: 8px; border-top: 1px solid var(--line); }
  .agent-tool-file {
    display: inline-block; max-width: 100%; height: 22px; padding: 0 7px; border-radius: var(--r-sm);
    background: var(--bg-panel); box-shadow: var(--ctl-edge); color: var(--tx);
    font: var(--fs-xs)/22px var(--f-mono); white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
    animation: agent-pop-in 250ms cubic-bezier(.23, 1, .32, 1) calc(var(--i) * 70ms) both;
  }

  @media (prefers-reduced-motion: reduce) {
    .agent-tool-activity::details-content, .agent-tool-chevron, .agent-tool-row-chevron, .agent-tool-output { transition: none; }
    .agent-tool-details > div, .agent-tool-file { animation: none; }
    .agent-tool-spinner { animation: none; border-top-color: var(--line-2); }
  }
</style>
