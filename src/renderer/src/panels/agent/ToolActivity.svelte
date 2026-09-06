<script lang="ts">
  import type { ToolsRow } from './activity-rows';

  let { row, animated = false }: { row: ToolsRow & { pulsing?: boolean }; animated?: boolean } = $props();

  const isRunning = $derived(row.status === 'running');
  const summary = $derived(isRunning ? (row.currentLabel || row.label) : row.label);
  const statusLabel = (status: ToolsRow['details'][number]['status']): string => {
    if (status === 'running') return 'Running';
    if (status === 'error') return 'Failed';
    if (status === 'continued') return 'Continued';
    return 'Done';
  };
</script>

<details
  class="agent-trace-tool agent-tool-activity"
  class:is-error={row.status === 'error'}
  class:is-partial={row.status === 'partial'}
  class:is-pulsing={animated && row.pulsing}
  open={isRunning}
  title={row.detail?.length ? row.detail.join('\n') : undefined}
>
  <summary class:has-status={isRunning || row.status === 'error' || row.status === 'partial'}>
    <svg class="agent-tool-wrench" viewBox="0 0 256 256" aria-hidden="true" focusable="false"><path d="M226.76,69a8,8,0,0,0-12.84-2.88l-40.3,37.19-17.23-3.7-3.7-17.23,37.19-40.3A8,8,0,0,0,187,29.24,72,72,0,0,0,88,96,72.34,72.34,0,0,0,94,124.94L33.79,177.79c-.15.12-.29.26-.43.39a32,32,0,0,0,45.26,45.26c.13-.14.27-.28.39-.43L131.06,162A72,72,0,0,0,232,96,71.56,71.56,0,0,0,226.76,69Zm-71,89a56.14,56.14,0,0,1-21.31-4.18,8,8,0,0,0-9,1.87l-.29.3-57.14,65a16,16,0,0,1-22.62-22.62l65-57.14.3-.29a8,8,0,0,0,1.87-9A56,56,0,0,1,168.72,45.53L138.83,77.9a8,8,0,0,0-1.94,7.1L142.83,113a8,8,0,0,0,6.14,6.14l28,6a8,8,0,0,0,7.1-1.94l32.37-29.89A56.09,56.09,0,0,1,155.76,158Z" /></svg>
    <span>{summary}</span>
    {#if isRunning}
      <em>Working</em>
    {:else if row.status === 'error'}
      <em>Failed</em>
    {:else if row.status === 'partial'}
      <em>{row.successCount} completed · {row.failedCount} failed</em>
    {/if}
    {#if row.details.length}
      <svg class="agent-tool-chevron" viewBox="0 0 12 12" aria-hidden="true"><path d="m3 4 3 3 3-3" /></svg>
    {/if}
  </summary>

  {#if row.details.length}
    <div class="agent-tool-details" aria-label="Tool activity details">
      {#each row.details as detail, index (`${detail.id}-${index}`)}
        <div class:is-failed={detail.status === 'error'} class:is-running={detail.status === 'running'}>
          <i aria-hidden="true"></i>
          <span>{detail.label}</span>
          <em>{statusLabel(detail.status)}</em>
        </div>
      {/each}
    </div>
  {/if}
</details>

<style>
  .agent-tool-activity {
    position: relative;
    display: block;
    overflow: hidden;
    border-radius: var(--r-sm);
    border: 1px solid var(--line);
    background: var(--ink-1);
    animation: none;
    color: var(--tx-2);
  }
  .agent-tool-activity.is-pulsing {
    background: color-mix(in srgb, var(--accent) 5%, transparent);
  }
  .agent-tool-activity.is-pulsing::before {
    content: '';
    position: absolute;
    inset: 0;
    z-index: 0;
    background: linear-gradient(90deg, transparent, color-mix(in srgb, var(--accent) 14%, transparent), transparent);
    background-size: 200% 100%;
    animation: agent-tool-sweep 1.8s ease-in-out infinite;
    pointer-events: none;
  }
  summary {
    position: relative;
    z-index: 1;
    display: grid;
    grid-template-columns: 16px minmax(0, 1fr) 12px;
    grid-template-rows: auto;
    align-items: start;
    gap: 3px 7px;
    min-width: 0;
    padding: 7px 8px;
    cursor: pointer;
    list-style: none;
    user-select: text;
    -webkit-user-select: text;
  }
  summary:focus-visible { outline: 1px solid var(--accent); outline-offset: -2px; border-radius: var(--r-sm); }
  summary::-webkit-details-marker { display: none; }
  summary.has-status { grid-template-rows: auto auto; }
  summary > span { grid-column: 2; grid-row: 1; min-width: 0; overflow-wrap: anywhere; }
  summary > em { grid-column: 2; grid-row: 2; text-align: left; line-height: 1.4; }
  summary > em,
  .agent-tool-details em { flex: none; font-size: 11px; font-style: normal; opacity: .72; }
  .agent-tool-wrench { grid-column: 1; grid-row: 1; margin-top: 1px; width: 16px; height: 16px; flex: none; fill: currentColor; color: var(--tx-4); }
  .agent-tool-chevron { grid-column: 3; grid-row: 1 / -1; align-self: center; width: 12px; height: 12px; flex: none; fill: none; stroke: currentColor; stroke-width: 1.4; transition: transform var(--dur-2) var(--ease); }
  details[open] .agent-tool-chevron { transform: rotate(180deg); }
  .agent-tool-details {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: column;
    gap: 5px;
    margin: 0 8px 8px 15px;
    padding: 4px 0 2px 12px;
    border-left: 1px solid var(--line-2);
    color: var(--tx-2);
    font-size: var(--fs-xs);
    font-weight: var(--fw-regular);
    user-select: text;
    -webkit-user-select: text;
  }
  .agent-tool-details > div { display: flex; align-items: baseline; gap: 8px; min-width: 0; }
  .agent-tool-details span { min-width: 0; flex: 1; overflow-wrap: anywhere; }
  .agent-tool-details i { width: 5px; height: 5px; flex: none; border-radius: 50%; background: currentColor; opacity: .55; }
  .agent-tool-details .is-running { color: var(--tx-2); }
  .agent-tool-details .is-failed { color: var(--danger); }
  .agent-tool-activity.is-error { color: var(--danger); }
  .agent-tool-activity.is-partial { color: var(--tx-2); }
  .agent-tool-activity.is-partial .agent-tool-wrench { color: var(--tx-4); }

  @keyframes agent-tool-sweep { to { background-position: -200% 0; } }
  @media (prefers-reduced-motion: reduce) {
    .agent-tool-activity.is-pulsing::before { animation: none; transform: none; opacity: .55; }
    .agent-tool-chevron { transition: none; }
  }
</style>
