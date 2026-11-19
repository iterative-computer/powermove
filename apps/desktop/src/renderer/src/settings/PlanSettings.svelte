<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { formatBytes, usagePercent, type PlanDto, type ShareDto } from '../../../shared/aerolite';
  import { cloudBridge, openSignIn, subscribeAccount, type CloudUser } from '../cloud/account';
  import { openBillingPortal, openUpgrade, planError, refreshPlan, shareCurrentProject, subscribePlan } from '../cloud/plan';

  let { PM, active }: { PM: Record<string, any>; active: boolean } = $props();

  let account = $state<CloudUser | null>(null);
  let plan = $state<PlanDto | null>(null);
  let shares = $state<ShareDto[]>([]);
  let busy = $state(false);
  let error = $state('');
  let note = $state('');

  const offAccount = subscribeAccount(user => { account = user; });
  const offPlan = subscribePlan(next => { plan = next; });
  onDestroy(() => { offAccount(); offPlan(); });

  const day = (iso: string | null | undefined) => iso ? new Date(iso).toLocaleDateString(undefined, { month: 'short', day: 'numeric' }) : '';
  const money = (cents: number) => `$${(cents / 100).toFixed(cents % 100 ? 2 : 0)}`;

  async function load(): Promise<void> {
    if (!account) return;
    busy = true;
    try {
      await refreshPlan();
      error = planError();
      const listed = await cloudBridge()?.shares?.();
      shares = listed?.ok ? listed.value.filter(item => !item.revokedAt) : [];
    } finally { busy = false; }
  }
  async function act(run: () => Promise<string | void>): Promise<void> {
    if (busy) return;
    busy = true; error = ''; note = '';
    try { error = (await run()) || ''; } finally { busy = false; }
  }
  async function share(): Promise<void> {
    await act(async () => {
      const result = await shareCurrentProject(PM);
      if (!result) return;
      if ('error' in result) return result.error;
      note = 'Link copied. Anyone with it can view a read-only page of this project.';
    });
    await load();
  }
  async function revoke(item: ShareDto): Promise<void> {
    const confirmed = await PM.confirm?.({ message: `Remove the link to “${item.title}”?`, detail: 'The page stops working for everyone. Removing a link doesn’t give back its share.', confirmLabel: 'Remove Link', destructive: true });
    if (!confirmed) return;
    await act(async () => {
      const result = await cloudBridge()?.revokeShare?.({ id: item.id });
      if (result && !result.ok) return result.error.detail ?? 'Couldn’t remove the link.';
    });
    await load();
  }

  $effect(() => { if (active && account) untrack(() => { void load(); }); });

  const free = $derived(!plan || plan.plan.tier === 'free');
  const shareSummary = $derived.by(() => {
    if (!plan) return '';
    const quota = plan.plan.shares;
    if (quota.kind === 'unlimited') return 'Unlimited links.';
    if (quota.kind === 'lifetime') return plan.shares.canCreate ? 'Your plan includes one link.' : 'You’ve used your included link.';
    return plan.shares.canCreate ? 'One new link each week. This week’s is available.' : `One new link each week. Your next is available ${day(plan.shares.nextAvailableAt)}.`;
  });
  const PROVIDERS = [{ id: 'anthropic', label: 'Claude' }, { id: 'openai', label: 'OpenAI' }] as const;
</script>

<header class="sg-heading">
  <h2>Plan & Usage</h2>
  <p>Your Powermove plan, built-in model allowances, share links and cloud storage.</p>
</header>
{#if !cloudBridge()}
  <p class="settings-note">Plans are available in the desktop app.</p>
{:else if !account}
  <section class="sg-section">
    <h3 class="sg-section-title">Plan</h3>
    <div class="sg-group">
      <div class="settings-row">
        <div class="settings-copy">
          <b>Sign in to see your plan</b>
          <span>Aerolite adds Claude and OpenAI models, cloud storage and weekly share links.</span>
        </div>
        <button class="btn" type="button" onclick={() => openSignIn()}>Sign In…</button>
      </div>
    </div>
  </section>
{:else}
  <section class="sg-section">
    <h3 class="sg-section-title">Plan</h3>
    <div class="sg-group" aria-busy={busy}>
      <div class="settings-row">
        <div class="settings-copy">
          <b>{plan ? plan.plan.tier === 'free' ? 'Free' : `${plan.plan.label} · ${money(plan.plan.priceCents)}/month` : 'Checking your plan…'}</b>
          <span>{!plan ? 'One moment.'
            : free ? 'Bring your own agent. Aerolite adds built-in Claude and OpenAI models.'
            : plan.subscription?.cancelAtPeriodEnd ? `Ends ${day(plan.subscription.renewsAt)}. You keep everything until then.`
            : plan.subscription?.status === 'past_due' ? 'Your last payment didn’t go through. Update your card to keep Aerolite.'
            : `Renews ${day(plan.subscription?.renewsAt)}.`}</span>
        </div>
        {#if plan && free}
          <button class="btn pri" type="button" disabled={busy || !plan.billingConfigured} onclick={() => openUpgrade()}>Upgrade…</button>
        {:else if plan}
          <button class="btn" type="button" disabled={busy} onclick={() => void act(openBillingPortal)}>Manage Billing…</button>
        {/if}
      </div>
    </div>
    {#if error}<p class="settings-note" role="alert">{error}</p>{/if}
  </section>

  {#if plan?.usage}
    <section class="sg-section">
      <h3 class="sg-section-title">Built-in models</h3>
      <div class="sg-group">
        {#each PROVIDERS as provider (provider.id)}
          {@const usage = plan.usage[provider.id]}
          {@const models = plan.plan.models.filter(model => model.provider === provider.id).map(model => model.label).join(', ')}
          <div class="settings-row">
            <div class="settings-copy plan-usage">
              <b>{provider.label}</b>
              <span>{models}</span>
              <div class="plan-meter" role="meter" aria-label={`${provider.label} usage this month`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(usage.period * 100)}>
                <i style:width={`${Math.max(usage.period > 0 ? 2 : 0, Math.min(100, usage.period * 100))}%`} class:full={usage.period >= 1}></i>
              </div>
              <span>{usage.period >= 1 ? `This month’s allowance is used. It refills ${day(usage.periodResetsAt)}.`
                : usage.day >= 1 ? `Today’s allowance is used. It refills tomorrow.`
                : `${usagePercent(usage.period)} used this month · refills ${day(usage.periodResetsAt)}`}</span>
            </div>
          </div>
        {/each}
        <div class="settings-row">
          <div class="settings-copy">
            <b>Use in the agent panel</b>
            <span>Choose Aerolite in the agent’s model picker.</span>
          </div>
          <button class="btn" type="button" onclick={() => { PM.AgentUI?.setProvider?.('aerolite'); PM.SettingsUI?.close?.(); }}>Use Aerolite</button>
        </div>
      </div>
    </section>
  {/if}

  {#if plan}
    <section class="sg-section">
      <h3 class="sg-section-title">Share links</h3>
      <div class="sg-group">
        <div class="settings-row">
          <div class="settings-copy">
            <b>Share this project</b>
            <span>{shareSummary}</span>
          </div>
          <button class="btn" type="button" disabled={busy || !PM.proj} onclick={() => void share()}>Copy Link…</button>
        </div>
        {#each shares as item (item.id)}
          <div class="settings-row">
            <div class="settings-copy">
              <b>{item.title}</b>
              <span>Shared {day(item.createdAt)} · {item.viewCount === 1 ? '1 view' : `${item.viewCount} views`}</span>
            </div>
            <button class="btn" type="button" onclick={() => void navigator.clipboard?.writeText(item.url).then(() => { note = 'Link copied.'; })}>Copy</button>
            <button class="btn" type="button" disabled={busy} onclick={() => void revoke(item)}>Remove…</button>
          </div>
        {/each}
      </div>
      {#if note}<p class="settings-note" role="status">{note}</p>{/if}
    </section>

    <section class="sg-section">
      <h3 class="sg-section-title">Cloud storage</h3>
      <div class="sg-group">
        <div class="settings-row">
          <div class="settings-copy plan-usage">
            <b>{formatBytes(plan.storage.usedBytes)} of {formatBytes(plan.storage.limitBytes)} used</b>
            <div class="plan-meter" role="meter" aria-label="Cloud storage used" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(100 * plan.storage.usedBytes / Math.max(1, plan.storage.limitBytes))}>
              <i style:width={`${Math.min(100, 100 * plan.storage.usedBytes / Math.max(1, plan.storage.limitBytes))}%`}></i>
            </div>
          </div>
        </div>
      </div>
    </section>
  {/if}
{/if}

<style>
  .plan-usage { flex: 1; }
  .plan-meter { width: 100%; max-width: 320px; height: 4px; margin: 6px 0 2px; border-radius: 2px; background: var(--ink-2); overflow: hidden; }
  .plan-meter i { display: block; height: 100%; border-radius: inherit; background: var(--accent); transition: width var(--dur-2) var(--ease); }
  .plan-meter i.full { background: var(--warning); }
</style>
