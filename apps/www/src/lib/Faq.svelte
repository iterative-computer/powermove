<script lang="ts">
  import type { Qa } from './site';

  let { items, title = 'Questions and answers.', lead = '' }: { items: Qa[]; title?: string; lead?: string } = $props();
</script>

<!-- Every answer stays open: short, visible text is what readers skim and what
     answer engines quote. -->
<section class="faq" id="faq" aria-labelledby="faq-title">
  <h2 id="faq-title" class="rows-title"><span class="strong">{title}</span>{#if lead}{' '}{lead}{/if}</h2>
  <dl class="faq-list">
    {#each items as item (item.q)}
      <div class="faq-item">
        <dt>{item.q}</dt>
        <dd>{item.a}</dd>
      </div>
    {/each}
  </dl>
</section>
