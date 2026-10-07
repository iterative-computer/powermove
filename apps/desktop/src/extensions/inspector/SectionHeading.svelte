<script lang="ts">
  import type { Snippet } from 'svelte';
  import { inspectorContext } from './context';

  let { title, empty = false, children }: { title: string; empty?: boolean; children: Snippet } = $props();
  const { api } = inspectorContext();
  const { Section } = api.ui.controls;
</script>

<div class="inspector-section-heading" class:is-empty={empty} data-empty-section={empty ? title : undefined}>
  <Section {api} {title}>{@render children()}</Section>
</div>

<style>
  .inspector-section-heading :global(.sec) {
    margin-top: 8px;
    border-top: 1px solid var(--section-line);
  }
  .inspector-section-heading.is-empty :global(.sec) {
    height: 33px;
    margin-top: 0;
    color: var(--tx);
    font-weight: var(--fw-regular);
  }
  .inspector-section-heading :global(.section-action) {
    display: grid;
    flex: none;
    width: 24px;
    height: 24px;
    padding: 0;
    place-items: center;
    border: 0;
    border-radius: var(--r-xs);
    background: transparent;
    color: var(--tx-3);
    font-size: 18px;
  }
  .inspector-section-heading :global(.section-action:hover) { background: var(--ink-1); color: var(--tx); }
  .inspector-section-heading :global(.section-action:focus-visible) { outline: 2px solid var(--accent); outline-offset: 2px; }
  .inspector-section-heading :global(.section-action svg) { width: 13px; height: 13px; }
</style>
