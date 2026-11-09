<script lang="ts">
  import type { ExtensionRecord, MenuContribution, PowermoveAPI } from 'powermove';

  import ModRow from './ModRow.svelte';

  let { api }: { api: PowermoveAPI } = $props();

  const QUIET_STATES = ['ok', 'disabled', 'replaced'];

  let records = $state.raw<ExtensionRecord[]>(read());

  const mine = $derived(records.filter((record) => record.scope !== 'builtin'));
  const builtin = $derived(records.filter((record) => record.scope === 'builtin'));
  const attention = $derived(records.filter((record) => !QUIET_STATES.includes(record.health.state)).length);
  const summary = $derived(
    `${records.length} ${records.length === 1 ? 'mod' : 'mods'}` + (attention > 0 ? ` · ${attention} needs attention` : '')
  );

  // Health and discovery updates can change records without loading a mod.
  $effect(() => {
    const stops = [
      api.events.on('extension:loaded', refresh),
      api.events.on('extension:unloaded', refresh),
      api.events.on('extensions:changed', refresh)
    ];
    return () => {
      for (const stop of stops) stop.dispose();
    };
  });

  function read(): ExtensionRecord[] {
    try {
      return api.extensions.list();
    } catch (error) {
      api.log('error', 'could not read the mod list', error);
      return [];
    }
  }

  function refresh(): void {
    records = read();
  }

  function nameOf(record: ExtensionRecord): string {
    return record.manifest?.name ?? record.id;
  }

  /** Optimistic local patch so the switch moves under the finger, not after IPC. */
  function patch(id: string, next: Partial<ExtensionRecord>): void {
    records = records.map((record) => (record.id === id ? { ...record, ...next } : record));
  }

  async function run(action: () => Promise<void> | void, failure: string): Promise<void> {
    try {
      await action();
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error ?? '');
      api.ui.toast(detail ? `${failure} ${detail}` : failure);
    }
    refresh();
  }

  function toggle(record: ExtensionRecord, next: boolean): void {
    patch(record.id, { enabled: next });
    void run(
      () => api.extensions.setEnabled(record.id, next),
      next ? `Could not turn on ${nameOf(record)}.` : `Could not turn off ${nameOf(record)}.`
    );
  }

  function setUp(record: ExtensionRecord): void {
    api.extensions.setUp(record.id);
  }

  function openMenu(record: ExtensionRecord, anchor: HTMLElement): void {
    const items: MenuContribution[] = [
      { label: 'Reload', run: () => void run(() => api.extensions.reload(record.id), `Could not reload ${nameOf(record)}.`) }
    ];
    if (record.scope === 'user' && record.manifest?.vars?.length) {
      items.push({ label: record.health.state === 'needs-setup' ? 'Set up…' : 'Variables…', run: () => setUp(record) });
    }
    if (record.scope !== 'builtin') {
      items.push({
        label: 'Show in Finder',
        run: () => void run(() => api.extensions.reveal(record.id), `Could not show ${nameOf(record)}.`)
      });
      items.push('-');
      items.push({ label: 'Remove…', run: () => void remove(record) });
    }
    api.ui.menu(anchor, items);
  }

  async function remove(record: ExtensionRecord): Promise<void> {
    const name = nameOf(record);
    const ok = await api.ui.confirm(`Remove ${name}?`, 'It is deleted from this computer. You can always ask for it again.');
    if (!ok) return;
    await run(() => api.extensions.remove(record.id), `Could not remove ${name}.`);
  }
</script>

<div class="mods">
  <div class="summary">{summary}</div>

  <section>
    <h2 class="group">Added by you</h2>
    {#if mine.length === 0}
      <p class="empty">Ask the agent to change anything about Powermove — it shows up here.</p>
    {:else}
      <div class="list" role="list">
        {#each mine as record (record.id)}
          <ModRow {record} onToggle={toggle} onMenu={openMenu} onSetup={setUp} />
        {/each}
      </div>
    {/if}
  </section>

  {#if builtin.length > 0}
    <section>
      <h2 class="group">Built in</h2>
      <div class="list" role="list">
        {#each builtin as record (record.id)}
          <ModRow {record} quiet onToggle={toggle} onMenu={openMenu} />
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .mods {
    display: flex;
    flex-direction: column;
    gap: 12px;
    width: 100%;
    padding: 8px;
    font-family: var(--f-ui);
  }

  .summary {
    padding: 0 6px;
    color: var(--tx-3);
    font-size: var(--fs-xs);
    line-height: var(--lh-tight);
  }

  section {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .group {
    margin: 0 0 2px;
    padding: 0 6px;
    color: var(--tx-4);
    font-size: var(--fs-xs);
    font-weight: var(--fw-medium);
    line-height: var(--lh-tight);
  }

  .empty {
    margin: 0;
    padding: 2px 6px 6px;
    color: var(--tx-3);
    font-size: var(--fs-xs);
    line-height: var(--lh);
  }

  .list {
    display: flex;
    flex-direction: column;
  }
</style>
