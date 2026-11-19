<script lang="ts">
  import { onDestroy } from 'svelte';
  import { fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import iconLight from './icon-light.png';
  import iconDark from './icon-dark.png';
  import { formatBytes } from '../../../shared/aerolite';
  import { currentMe, openSignIn, subscribeAccount } from './account';
  import { refreshPlan, startCheckout, subscribePlan } from './plan';

  let { PM, reason, onclose }: { PM: Record<string, any>; reason?: string; onclose: () => void } = $props();

  /* Offer → the browser has Checkout open → active. Checkout happens in the
     browser; the sheet waits and picks up the new plan when Powermove gets
     focus again (plan.ts refreshes on focus). */
  type Step = 'offer' | 'browser' | 'done';
  let step = $state<Step>('offer');
  let signedIn = $state(!!currentMe());
  let busy = $state(false);
  let error = $state('');
  let price = $state(900);
  let storage = $state(1024 ** 3);

  const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
  const slide = { x: 12, duration: reduced() ? 0 : 180, easing: cubicOut };
  const LEAD: Record<string, string> = {
    agent: 'Built-in models come with Aerolite',
    share: 'Your free share link is used',
  };

  const offAccount = subscribeAccount(user => { signedIn = !!user; });
  const offPlan = subscribePlan(plan => {
    if (!plan) return;
    const offer = plan.upgrades.find(item => item.tier === 'aerolite');
    if (offer) { price = offer.priceCents; storage = offer.storageBytes; }
    if (plan.plan.tier !== 'free') step = 'done';
  });
  onDestroy(() => { offAccount(); offPlan(); });

  async function upgrade(): Promise<void> {
    if (!signedIn) { onclose(); openSignIn('sign-up'); return; }
    if (busy) return;
    busy = true; error = '';
    try {
      error = await startCheckout();
      if (!error) step = 'browser';
    } finally { busy = false; }
  }
  async function check(): Promise<void> {
    busy = true;
    try { await refreshPlan(); } finally { busy = false; }
  }
  function useModels(): void {
    PM.AgentUI?.setProvider?.('aerolite');
    onclose();
  }
  const dollars = $derived(`$${(price / 100).toFixed(price % 100 ? 2 : 0)}`);
</script>

<div class="acct-sheet upg" tabindex="-1" data-autofocus>
  {#key step}
    <div class="acct-step" in:fly={slide}>
      <img class="acct-mark is-light" src={iconLight} alt="" width="48" height="48" draggable="false" />
      <img class="acct-mark is-dark" src={iconDark} alt="" width="48" height="48" draggable="false" />
      {#if step === 'done'}
        <header class="acct-head">
          <h2>You’re on Aerolite</h2>
          <p>Claude and OpenAI models are ready in the agent panel. Each has its own monthly allowance.</p>
        </header>
        <div class="acct-providers">
          <button class="btn pri acct-submit" type="button" onclick={useModels}>Use Aerolite Models</button>
          <button class="btn acct-provider" type="button" onclick={() => { onclose(); PM.SettingsUI?.open?.('plan'); }}>See Plan & Usage</button>
        </div>
      {:else}
        <header class="acct-head">
          <span class="upg-eyebrow">Aerolite</span>
          <h2>{reason && LEAD[reason] ? LEAD[reason] : 'Powermove, with models built in'}</h2>
          <p>Skip the API keys. Your agent runs on Claude and OpenAI models through your Powermove account.</p>
        </header>
        <ul class="upg-list">
          <li><b>Claude Sonnet 5.5 and Haiku 5.5</b><span>With their own monthly allowance</span></li>
          <li><b>GPT-6.2 Sol and GPT-6 Luna</b><span>A separate allowance, so one never uses up the other</span></li>
          <li><b>{formatBytes(storage)} of cloud storage</b><span>For your share pages</span></li>
          <li><b>A new share link every week</b><span>A read-only page for any project</span></li>
        </ul>
        {#if step === 'browser'}
          <div class="acct-wait" role="status">
            <svg class="acct-spinner" viewBox="0 0 16 16" aria-hidden="true">{#each Array(12) as _, i}<line x1="8" y1="1.5" x2="8" y2="4" opacity={(i + 1) / 12} transform={`rotate(${i * 30} 8 8)`} />{/each}</svg>
            <span>Finish checkout in your browser.</span>
            <button class="btn" type="button" disabled={busy} onclick={() => void check()}>Check Again</button>
          </div>
        {:else}
          <div class="acct-providers">
            <button class="btn pri acct-submit" type="button" disabled={busy} onclick={() => void upgrade()}>
              {busy ? 'Opening checkout…' : signedIn ? `Upgrade for ${dollars}/month` : 'Sign In to Upgrade'}
            </button>
          </div>
        {/if}
        {#if error}<p class="acct-status is-wrong upg-error" role="alert">{error}</p>{/if}
        <div class="acct-foot">
          <p>Billed monthly through Stripe. Cancel any time.</p>
          <button class="btn" type="button" onclick={onclose}>Not Now</button>
        </div>
      {/if}
    </div>
  {/key}
</div>
