<script lang="ts">
  import { durationLabel } from './activity-rows';
  import { toRichWords } from './rich-words';
  import { revealText } from './text-reveal';

  /* Reasoning, harness ThinkingState style: a quiet header that shimmers
     "Thinking" while the model reasons and settles to "Thought for 4s"; the
     prose itself hangs under a hairline and stays one click away. Live
     thoughts are open so the reader sees the reasoning arrive; settled ones
     close so a long run reads as a list of headers, not a wall of musing. */

  let {
    label,
    live = false,
    startedAt,
    endedAt,
    animated = false
  }: { label: string; live?: boolean; startedAt?: number; endedAt?: number; animated?: boolean } = $props();

  const words = $derived(toRichWords(label));
  const isAction = $derived(words.some((word) => word.b));
  const span = $derived(startedAt !== undefined && endedAt !== undefined ? durationLabel(endedAt - startedAt) : '');
  const heading = $derived(live ? 'Thinking' : span ? `Thought for ${span}` : 'Thought');
</script>

<details class="agent-trace-thought agent-thinking" class:is-live={live} class:is-action={isAction} open={live}>
  <summary>
    <svg class="agent-thinking-mark" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 2l2.4 7.2L22 12l-7.6 2.8L12 22l-2.4-7.2L2 12l7.6-2.8z" /></svg>
    <span class="agent-thinking-heading" class:shimmer-text={live && animated} role="status">{heading}</span>
    <svg class="agent-thinking-chevron" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 9l6 6 6-6" /></svg>
  </summary>
  <p class="agent-thinking-body">{#each words as word, wi (wi)}<span use:revealText={animated && live && Boolean(word.w.trim())} class={word.c ? 'agent-trace-code' : ''}>{word.w}</span>{/each}</p>
</details>

<style>
  .agent-thinking {
    display: block;
    min-width: 0;
    color: var(--tx-3);
    interpolate-size: allow-keywords;
  }
  .agent-thinking::details-content {
    display: block;
    block-size: 0;
    overflow: clip;
    opacity: 0;
    transition: block-size 320ms cubic-bezier(.23, 1, .32, 1), opacity 240ms ease-out, content-visibility 320ms allow-discrete;
  }
  .agent-thinking[open]::details-content { block-size: auto; opacity: 1; }
  summary {
    display: flex;
    align-items: center;
    gap: 6px;
    width: fit-content;
    max-width: 100%;
    margin: 0 -6px;
    padding: 4px 6px;
    border-radius: var(--r-sm);
    list-style: none;
    cursor: pointer;
    font-size: 12.5px;
    font-weight: var(--fw-medium);
    line-height: 1.45;
    color: var(--tx-2);
    transition: background var(--dur-2), color var(--dur-2);
  }
  summary:hover { background: var(--ink-1); color: var(--tx); }
  summary:focus-visible { outline: 1px solid var(--accent); outline-offset: -2px; }
  summary::-webkit-details-marker { display: none; }
  .agent-thinking-mark { width: 13px; height: 13px; flex: none; fill: var(--tx-4); transition: fill 200ms; }
  .is-live > summary .agent-thinking-mark { fill: var(--tx-2); }
  .agent-thinking-heading { min-width: 0; white-space: nowrap; }
  .agent-thinking-chevron {
    width: 12px; height: 12px; flex: none; fill: none; stroke: var(--tx-4); stroke-width: 2.2; stroke-linecap: round; stroke-linejoin: round;
    transition: transform 240ms var(--ease);
  }
  details[open] > summary .agent-thinking-chevron { transform: rotate(180deg); }
  .agent-thinking-body {
    margin: 2px 0 4px 6px;
    padding: 2px 0 2px 13px;
    border-left: 1px solid var(--line-2);
    color: var(--tx-3);
    font-size: 12.5px;
    line-height: 1.55;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .is-action .agent-thinking-body { color: var(--tx-2); font-weight: var(--fw-medium); }
  @media (prefers-reduced-motion: reduce) {
    .agent-thinking::details-content, .agent-thinking-chevron { transition: none; }
  }
</style>
