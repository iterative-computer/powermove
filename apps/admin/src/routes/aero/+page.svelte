<script lang="ts">
  import { enhance } from '$app/forms';
  import { time } from '$lib/format';

  let { data, form } = $props();
  const o = $derived(data.overview);
  const usd = (n: number) => `$${n < 10 ? n.toFixed(4) : n.toFixed(2)}`;
  const int = (n: number) => Math.round(n).toLocaleString('en');
  const pct = (n: number) => `${(n * 100).toFixed(1)}%`;
  const reason = (r: string) => r.replace(/^(charge|invoice)\./, '').split('_').join(' ');
  const peak = $derived(Math.max(...o.daily.map((d) => d.costUsd), 0.000001));
  const ceilingShare = $derived(o.ceiling.ceilingUsd > 0 ? Math.min(1, o.ceiling.todayUsd / o.ceiling.ceilingUsd) : 0);
  const windows = $derived([{ label: 'Today', u: o.today }, { label: 'Last 30 days', u: o.last30 }]);
  let busy = $state(false);
  const submit = () => {
    busy = true;
    return async ({ update }: { update: () => Promise<void> }) => {
      await update();
      busy = false;
    };
  };
</script>

<svelte:head><title>Aero & Billing · Powermove Admin</title></svelte:head>

<header class="heading">
  <h1>Aero & Billing</h1>
  <p>Provider cost and usage from the credit ledger, subscriptions from Stripe's webhooks. Updated {time(o.generatedAt)}.</p>
</header>

{#each windows as w (w.label)}
  <section class="section">
    <h2 class="section-title">{w.label}</h2>
    <div class="stats">
      <div><b>{usd(w.u.costUsd)}</b><span>Provider cost</span></div>
      <div><b>{int(w.u.credits)}</b><span>Credits spent</span></div>
      <div><b>{int(w.u.calls)}</b><span>Calls{w.u.estimatedCalls ? ` · ${int(w.u.estimatedCalls)} estimated` : ''}</span></div>
      <div><b>{pct(w.u.cacheHitRatio)}</b><span>Cache hit ratio</span></div>
    </div>
    <div class="group">
      <div class="row"><div class="copy"><b>Input tokens</b></div><div class="value">{int(w.u.inputTokens)}</div></div>
      <div class="row"><div class="copy"><b>Cached (read)</b></div><div class="value">{int(w.u.cachedTokens)}</div></div>
      <div class="row"><div class="copy"><b>Cache writes</b></div><div class="value">{int(w.u.cacheWriteTokens)}</div></div>
      <div class="row"><div class="copy"><b>Output tokens</b></div><div class="value">{int(w.u.outputTokens)}</div></div>
    </div>
  </section>
{/each}

<section class="section">
  <h2 class="section-title">Spend today against the ceiling</h2>
  <div class="group">
    <div class="row">
      <div class="copy"><b>{usd(o.ceiling.todayUsd)} of {usd(o.ceiling.ceilingUsd)}</b><span class="wrap">Spends and holds created today (UTC), as admission counts them. At the ceiling Aero answers 503.</span></div>
      <div class="meter" role="img" aria-label="{pct(ceilingShare)} of the daily ceiling"><i style:width="{ceilingShare * 100}%"></i></div>
    </div>
  </div>
</section>

<section class="section">
  <h2 class="section-title">Cost per day, last 30 days</h2>
  <div class="group chart">
    <div class="bars" role="img" aria-label="Daily provider cost">
      {#each o.daily as d (d.date)}
        <div class="bar" title="{d.date}: {usd(d.costUsd)}, {int(d.calls)} calls"><i style:height="{(d.costUsd / peak) * 100}%"></i></div>
      {/each}
    </div>
    <div class="axis"><span>{o.daily[0]?.date}</span><span>Peak {usd(peak)}</span><span>{o.daily.at(-1)?.date}</span></div>
  </div>
</section>

<section class="section">
  <h2 class="section-title">Per model, last 30 days</h2>
  <div class="group">
    {#each o.models as m (m.model)}
      <div class="row"><div class="copy"><b>{m.model}</b><span>{int(m.calls)} calls · {int(m.inputTokens)} in · {pct(m.cacheHitRatio)} cached · {int(m.outputTokens)} out</span></div><div class="value">{usd(m.costUsd)}</div></div>
    {:else}
      <div class="empty">No calls yet.</div>
    {/each}
  </div>
</section>

<section class="section">
  <h2 class="section-title">Subscriptions</h2>
  <div class="stats">
    <div><b>{int(o.subscriptions.active)}</b><span>Active</span></div>
    <div><b>${(o.subscriptions.mrrCents / 100).toFixed(2)}</b><span>MRR</span></div>
    <div><b>{int(o.subscriptions.new30)}</b><span>New, 30 days</span></div>
    <div><b>{int(o.subscriptions.cancelling)} · {int(o.subscriptions.pastDue)}</b><span>Cancelling · past due</span></div>
  </div>
</section>

<section class="section">
  <h2 class="section-title">Top accounts by spend, last 30 days</h2>
  <div class="group">
    {#each o.topAccounts as t (t.user.id)}
      <div class="row"><div class="copy"><b><a href="/users/{encodeURIComponent(t.user.id)}">{t.user.email ?? t.user.id}</a></b><span>{int(t.calls)} calls · {t.creditsLeft === null ? 'no current plan' : `${int(t.creditsLeft)} credits left`}</span></div><div class="value">{usd(t.costUsd)}</div></div>
    {:else}
      <div class="empty">No spend yet.</div>
    {/each}
  </div>
</section>

<section class="section">
  <h2 class="section-title">Blocked accounts</h2>
  <div class="group">
    {#each o.blocked as b (b.user.id)}
      <form class="row" method="POST" action="?/unblock" use:enhance={submit}>
        <div class="copy"><b><a href="/users/{encodeURIComponent(b.user.id)}">{b.user.email ?? b.user.id}</a></b><span class="wrap">{b.reason} · {time(b.createdAt)}</span></div>
        <input type="hidden" name="userId" value={b.user.id} />
        <button class="btn" type="submit" disabled={busy}>Unblock</button>
      </form>
    {:else}
      <div class="empty">No account is blocked.</div>
    {/each}
  </div>
  {#if form?.message}<p class="note error" role="alert">{form.message}</p>{/if}
  <p class="note">An account is blocked when a settled provider cost passed its hold. Unblocking is recorded in the moderation log.</p>
</section>

<section class="section">
  <h2 class="section-title">Estimated spends (billed at the full hold), last 30 days</h2>
  <div class="group">
    {#each o.estimated as e, i (i)}
      <div class="row"><div class="copy"><b>{e.user.email ?? e.user.id}</b><span>{e.model ?? 'unknown'} · {time(e.createdAt)}</span></div><div class="value">{usd(e.costUsd)}</div></div>
    {:else}
      <div class="empty">None.</div>
    {/each}
  </div>
</section>

<section class="section">
  <h2 class="section-title">Refunds and disputes</h2>
  <div class="group">
    {#each o.reversals as r (r.invoiceId)}
      <div class="row"><div class="copy"><b>{r.user.email ?? r.user.id}</b><span>{reason(r.reason)} · {time(r.createdAt)}</span></div><div class="value mono">{r.invoiceId}</div></div>
    {:else}
      <div class="empty">None.</div>
    {/each}
  </div>
</section>
