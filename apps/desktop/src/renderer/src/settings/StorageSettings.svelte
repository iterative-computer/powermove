<script lang="ts">
  import { untrack } from 'svelte';
  import { bridge } from '../kernel/bridge';
  import { BACKUP_BUDGETS, type StorageSettings, type StorageStatus } from '../../../shared/storage-management';
  import { formatBytes } from '../transcription/format';

  let { PM, active }: { PM: Record<string, any>; active: boolean } = $props();
  const transition = bridge()?.transition;
  let transitionStatus = $state<import('../../../shared/transition').TRStatus | null>(null);
  const host = bridge()?.storage;
  let status = $state<StorageStatus | null>(null);
  let busy = $state(false);
  let error = $state('');
  let result = $state('');
  async function run(action: () => Promise<void>): Promise<void> {
    if (busy) return;
    busy = true;
    error = '';
    try { await action(); }
    catch (cause) {
      error = cause instanceof Error ? cause.message.replace(/^Error invoking remote method '[^']+': (?:Error: )?/, '') : 'Unable to manage storage. Try again.';
    } finally { busy = false; }
  }
  function refresh(): Promise<void> {
    return run(async () => { if (host) status = await host.status(); });
  }
  function configure(settings: StorageSettings): Promise<void> {
    return run(async () => { if (host) { status = await host.configure(settings); result = ''; } });
  }
  async function cleanup(): Promise<void> {
    if (!host || busy || !status) return;
    const confirmed = await PM.confirm?.({
      message: 'Clean up storage?',
      detail: 'Remove older backups and temporary caches. The two newest backups per project and the two newest recovery snapshots in each group will stay. Projects, imported media, AI conversations and sign-ins will stay.',
      confirmLabel: 'Clean Up', destructive: true
    });
    if (!confirmed) return;
    await run(async () => {
      result = '';
      await PM.MediaStore?.removeLegacyDuplicates?.();
      const cleaned = await host.cleanup();
      status = cleaned.status;
      result = `Freed ${formatBytes(cleaned.freedBytes)}.${cleaned.warnings.length ? ` ${cleaned.warnings.join(' ')}` : ''}`;
    });
  }
  $effect(() => {
    if (!active || !transition) return;
    const refreshTransition = () => { void transition.request({ action: 'status' }).then(value => transitionStatus = value).catch(() => {}); };
    refreshTransition(); const timer = setInterval(refreshTransition, 2000);
    return () => clearInterval(timer);
  });
  $effect(() => { if (active && host) untrack(() => { void refresh(); }); });
</script>

<header class="sg-heading">
  <h2>Storage</h2>
  <p>Manage the space Powermove uses on this Mac.</p>
</header>
{#if !host}
  <p class="settings-note">Storage management is available in the desktop app.</p>
{:else}
  {#if transition}
    <section class="sg-section">
      <h3 class="sg-section-title">Native transition</h3>
      <div class="sg-group">
        <div class="settings-row">
          <div class="settings-copy">
            <b>{transitionStatus?.mediaComplete ? 'Media prepared' : 'Preparing your library'}</b>
            <span>{transitionStatus ? `${transitionStatus.done} of ${transitionStatus.total} media records verified.` : 'Checking preparation progress…'}</span>
          </div>
          <button class="btn" disabled={busy} onclick={() => run(async () => {
            transitionStatus = await transition.request({ action: 'secrets' });
          })}>Prepare Sign-ins</button>
        </div>
        <div class="settings-row">
          <div class="settings-copy"><b>Native app</b><span>Available after library preparation and signed migration verification.</span></div>
          <button class="btn" disabled>Switch to Native</button>
        </div>
      </div>
    </section>
  {/if}
  <section class="sg-section">
    <h3 class="sg-section-title">Space on this Mac</h3>
    <div class="sg-group" aria-busy={busy}>
      <div class="settings-row">
        <div class="settings-copy">
          <b>{status ? `${formatBytes(status.totalBytes)} used` : error ? 'Storage unavailable' : 'Calculating storage…'}</b>
          <span>{status ? `${formatBytes(status.reclaimableBytes)} can be freed from older backups and temporary caches.` : 'Checking backups, media and app data.'}</span>
        </div>
        <button class="btn" type="button" disabled={busy || !status?.reclaimableBytes} onclick={() => void cleanup()}>{busy ? 'Working…' : 'Clean Up'}</button>
      </div>
      {#each status?.categories ?? [] as category (category.id)}
        <div class="settings-row">
          <div class="settings-copy"><b>{category.label}</b></div>
          <span>{formatBytes(category.bytes)}</span>
        </div>
      {/each}
      <div class="settings-row">
        <div class="settings-copy"><b>Storage folder</b></div>
        <button class="btn" type="button" disabled={busy} onclick={() => void run(() => host.reveal())}>Show in Finder</button>
        <button class="btn" type="button" disabled={busy} onclick={() => void refresh()}>Refresh</button>
      </div>
    </div>
    {#if error}<p class="settings-note" role="alert">{error}</p>{/if}
    {#if result}<p class="settings-note" role="status">{result}</p>{/if}
  </section>
  {#if status}
    <section class="sg-section">
      <h3 class="sg-section-title">Automatic cleanup</h3>
      <div class="sg-group">
        <div class="settings-row">
          <div class="settings-copy">
            <b>Clean up automatically</b>
            <span>Remove older backups and expired recovery files, and keep temporary caches small.</span>
          </div>
          <button class="toggle" class:on={status.settings.automatic} type="button" disabled={busy} aria-pressed={status.settings.automatic} aria-label="Clean up automatically"
            onclick={() => void configure({ ...status!.settings, automatic: !status!.settings.automatic })}><i aria-hidden="true"></i></button>
        </div>
        <div class="settings-row">
          <div class="settings-copy">
            <b>Older backup budget</b>
            <span>New saves keep crash recovery without retaining full project copies. The two newest older backups per project will stay.</span>
          </div>
          <select class="settings-select" aria-label="Older backup budget" value={status.settings.backupBudgetGiB} disabled={busy}
            onchange={(event) => void configure({ ...status!.settings, backupBudgetGiB: Number(event.currentTarget.value) })}>
            {#each BACKUP_BUDGETS as budget}<option value={budget}>{budget} GB</option>{/each}
          </select>
        </div>
      </div>
      {#if status.backupBudgetExceeded}<p class="settings-note">The newest recovery copies exceed your budget. They will be kept.</p>{/if}
    </section>
  {/if}
{/if}
