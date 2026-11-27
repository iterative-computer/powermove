<script lang="ts">
  import { DISCORD } from '$lib/links';
  import { download } from '$lib/download.svelte';

  type Feature = { title: string; detail: string; soon?: boolean };
  type Plan = {
    id: 'free' | 'aerolite' | 'aero';
    name: string;
    price: string;
    period?: string;
    soon?: boolean;
    note: string;
    lead: string;
    features: Feature[];
  };

  const plans: Plan[] = [
    {
      id: 'free',
      name: 'Free',
      price: '$0',
      note: 'The whole editor, with your own agent.',
      lead: 'Includes',
      features: [
        { title: 'The whole editor', detail: 'Every tool, effect and export' },
        { title: 'Extensions', detail: 'Install from the Store, or have the agent build one' },
        { title: 'Your own agent', detail: 'Bring your Claude Code or Codex subscription' },
      ],
    },
    {
      id: 'aerolite',
      name: 'Aerolite',
      price: '$9.99',
      period: 'per month',
      note: 'Aero, our agent, with inference included.',
      lead: 'Everything in Free, plus',
      features: [
        { title: '1,000 credits every month', detail: 'Inference for Aero, run by Powermove' },
        { title: 'Review links', detail: 'Share a cut by link. Viewers comment at any frame', soon: true },
        { title: 'Cloud projects', detail: 'Keep your projects in the cloud', soon: true },
      ],
    },
    {
      id: 'aero',
      name: 'Aero',
      price: '$19.99',
      period: 'per month',
      soon: true,
      note: 'For editors who lean on Aero every day.',
      lead: 'Everything in Aerolite, plus',
      features: [
        { title: '2,500 credits every month', detail: '25% more credits per dollar than Aerolite' },
        { title: 'Early access', detail: 'New features reach you before everyone else' },
        { title: 'Priority support', detail: 'A direct line to the team' },
        { title: 'More cloud storage', detail: 'Room for bigger projects and more review links', soon: true },
      ],
    },
  ];

  let grid = $state<HTMLElement | undefined>();
  let shown = $state(false);

  $effect(() => {
    if (!grid) return;
    const observer = new IntersectionObserver(
      ([entry]) => { if (entry.isIntersecting) { shown = true; observer.disconnect(); } },
      { threshold: 0.25 },
    );
    observer.observe(grid);
    return () => observer.disconnect();
  });

  // The Aerolite glow leans toward the pointer, the way the hero's light pools behind the window.
  function lean(event: PointerEvent) {
    if (event.pointerType !== 'mouse') return;
    const card = event.currentTarget as HTMLElement;
    const box = card.getBoundingClientRect();
    card.style.setProperty('--mx', `${((event.clientX - box.left) / box.width) * 100}%`);
  }
  function settle(event: PointerEvent) {
    (event.currentTarget as HTMLElement).style.removeProperty('--mx');
  }
</script>

<!-- Icons: IconCheck and IconArrowUpRight from @pierre/icons 0.9.0 (Apache-2.0). -->
{#snippet check()}
  <svg class="feature-mark" viewBox="0 0 16 16" aria-hidden="true"><path d="M13.8551 3.17382C14.1733 3.439 14.2163 3.91192 13.9512 4.23013L6.86783 12.7301C6.73293 12.892 6.53616 12.9897 6.32565 12.9992C6.11515 13.0088 5.91034 12.9293 5.76134 12.7803L2.21967 9.23865C1.92678 8.94576 1.92678 8.47088 2.21967 8.17799C2.51256 7.8851 2.98744 7.8851 3.28033 8.17799L6.24127 11.1389L12.7988 3.26985C13.064 2.95164 13.5369 2.90865 13.8551 3.17382Z" fill="currentColor"/></svg>
{/snippet}

<section class="pricing" id="pricing" aria-labelledby="pricing-title">
  <h2 id="pricing-title" class="rows-title"><span class="strong">The editor is free.</span> Plans add our inference, and soon the cloud.</h2>

  <div class="plans" bind:this={grid} data-in={shown ? '' : undefined}>
    {#each plans as plan, i (plan.id)}
      <article class="plan" data-plan={plan.id} style:--i={i} aria-labelledby="plan-{plan.id}"
        onpointermove={plan.id === 'aerolite' ? lean : undefined}
        onpointerleave={plan.id === 'aerolite' ? settle : undefined}>
        {#if plan.id === 'aerolite'}<div class="plan-glow" aria-hidden="true"></div>{/if}

        <div class="plan-head">
          <h3 id="plan-{plan.id}" class="plan-name">{plan.name}</h3>
          {#if plan.soon}<span class="tag">Coming soon</span>{/if}
        </div>

        <p class="plan-price">
          <span class="amount tabular">{plan.price}</span>
          {#if plan.period}<span class="period">{plan.period}</span>{/if}
        </p>

        <p class="plan-note">{plan.note}</p>

        <div class="plan-cta">
          {#if plan.id === 'free'}
            <a class="pill dark" href={download.href}>Download for Mac</a>
          {:else if plan.id === 'aerolite'}
            <a class="pill light" href={download.href}>Get Aerolite</a>
          {:else}
            <a class="pill outline" href={DISCORD} target="_blank" rel="noopener">Hear first on Discord<svg class="ext" viewBox="0 0 16 16" aria-hidden="true"><path d="M5 4.75C5 4.33579 5.33579 4 5.75 4H11.25C11.6642 4 12 4.33579 12 4.75V10.25C12 10.6642 11.6642 11 11.25 11C10.8358 11 10.5 10.6642 10.5 10.25V6.56066L5.53033 11.5303C5.23744 11.8232 4.76256 11.8232 4.46967 11.5303C4.17678 11.2374 4.17678 10.7626 4.46967 10.4697L9.43934 5.5H5.75C5.33579 5.5 5 5.16421 5 4.75Z" fill="currentColor"/></svg></a>
          {/if}
        </div>

        <div class="plan-features">
          <p class="features-lead">{plan.lead}</p>
          <ul>
            {#each plan.features as feature (feature.title)}
              <li data-soon={feature.soon ? '' : undefined}>
                {@render check()}
                <span class="feature-text">
                  <span class="feature-title">{feature.title}{#if feature.soon}<span class="tag">Soon</span>{/if}</span>
                  <span class="feature-detail">{feature.detail}</span>
                </span>
              </li>
            {/each}
          </ul>
        </div>
      </article>
    {/each}
  </div>

  <p class="pricing-fine">Prices in US dollars. Credits pay for Aero’s inference.</p>
</section>
