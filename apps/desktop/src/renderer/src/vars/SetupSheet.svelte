<script lang="ts">
  import { onMount, tick } from 'svelte';
  import Icon from '../panels/Icon.svelte';
  import type { ExtensionRecord } from '../../../shared/extensions';
  import type { VarsBridge, VarsKeyStatus, VarsStatus } from '../../../shared/vars-ipc';
  import { usefulHint } from './hint';

  let { PM, record, bridge, onclose }: {
    PM: Record<string, any>;
    record: ExtensionRecord;
    bridge: VarsBridge | null;
    onclose: () => void;
  } = $props();

  /* One row per declared value. Stored values never come back to this
     window: a set row shows dots, Reveal asks main to show a native dialog,
     and Change swaps in an empty field. The eye toggle only ever shows what
     is being typed right now. */
  const name = $derived(record.manifest?.name ?? record.id);
  const uid = `vars-${Math.random().toString(36).slice(2, 8)}`;

  let status = $state<VarsStatus | null>(null);
  let drafts = $state<Record<string, string>>({});
  let editing = $state<Record<string, boolean>>({});
  let shown = $state<Record<string, boolean>>({});
  let busy = $state(false);
  let error = $state('');
  let root: HTMLElement;

  const keys = $derived(status?.keys ?? []);
  const dirty = $derived(keys.some((k) => (drafts[k.key] ?? '').length > 0));
  const waiting = $derived(status?.status === 'needs-setup');

  const message = (cause: unknown, fallback: string): string =>
    cause instanceof Error && cause.message ? cause.message.replace(/^Error invoking remote method '[^']+': (?:Error: )?/, '') : fallback;

  async function load(): Promise<void> {
    if (!bridge) {
      error = 'Values aren’t available in this window.';
      return;
    }
    try {
      status = await bridge.status({ id: record.id });
    } catch (cause) {
      error = message(cause, 'Unable to load values. Try again.');
    }
  }

  function fieldFor(k: VarsKeyStatus): boolean {
    return !k.set || !!editing[k.key];
  }

  async function change(k: VarsKeyStatus): Promise<void> {
    editing = { ...editing, [k.key]: true };
    await tick();
    root?.querySelector<HTMLInputElement>(`#${uid}-${k.key}`)?.focus();
  }

  async function reveal(k: VarsKeyStatus): Promise<void> {
    if (!bridge) return;
    error = '';
    try {
      await bridge.reveal({ id: record.id, key: k.key });
    } catch (cause) {
      error = message(cause, `Unable to show ${k.label}.`);
    }
  }

  async function remove(k: VarsKeyStatus): Promise<void> {
    if (!bridge || busy) return;
    busy = true;
    error = '';
    try {
      status = await bridge.delete({ id: record.id, key: k.key });
      editing = { ...editing, [k.key]: false };
      drafts = { ...drafts, [k.key]: '' };
    } catch (cause) {
      error = message(cause, `Unable to remove ${k.label}. Try again.`);
    } finally {
      busy = false;
    }
  }

  async function save(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!bridge || busy || !dirty) return;
    busy = true;
    error = '';
    try {
      for (const k of keys) {
        const value = drafts[k.key] ?? '';
        if (!value) continue;
        status = await bridge.set({ id: record.id, key: k.key, value });
        drafts = { ...drafts, [k.key]: '' };
        editing = { ...editing, [k.key]: false };
        shown = { ...shown, [k.key]: false };
      }
    } catch (cause) {
      error = message(cause, 'Unable to save. Try again.');
      return;
    } finally {
      busy = false;
    }
    if (status?.status === 'ok') {
      PM.toast?.(`Saved values for ${name}`, 2200, { kind: 'status' });
      onclose();
    }
  }

  onMount(() => {
    void load();
  });
</script>

<!-- svelte-ignore a11y_autofocus -->
<div class="vars-sheet" bind:this={root} tabindex="-1" autofocus>
  <header class="acct-head">
    <h2>{name}</h2>
    {#if waiting}<p>Turns on once required values are set.</p>{/if}
  </header>

  <form class="sg-column vars-form" novalidate onsubmit={save}>
    {#if keys.length}
      <div class="sg-group">
        {#each keys as k (k.key)}
          {@const hint = usefulHint(k.label, k.hint)}
          <div class="settings-row vars-row">
            <label class="settings-copy" for={`${uid}-${k.key}`}>
              <b>{k.label}{#if k.required}<span class="vars-req">Required</span>{/if}</b>
              {#if k.undecryptable}
                <span class="vars-attention">Needs re-entry</span>
              {:else if hint}
                <span>{hint}</span>
              {/if}
            </label>
            {#if fieldFor(k)}
              <span class="vars-field">
                <input
                  id={`${uid}-${k.key}`}
                  type={k.secret && !shown[k.key] ? 'password' : 'text'}
                  autocomplete="off"
                  autocapitalize="off"
                  spellcheck="false"
                  disabled={busy}
                  bind:value={drafts[k.key]}
                />
                {#if k.secret}
                  <button
                    type="button"
                    class="vars-eye"
                    aria-label={shown[k.key] ? `Hide ${k.label}` : `Show ${k.label}`}
                    aria-pressed={!!shown[k.key]}
                    onclick={() => (shown = { ...shown, [k.key]: !shown[k.key] })}
                  ><Icon {PM} name={shown[k.key] ? 'eyeoff' : 'eye'} /></button>
                {/if}
              </span>
            {:else}
              <span class="vars-stored">
                <span class="vars-dots" aria-label={`${k.label} is set`}>••••</span>
                <button type="button" class="acct-link" disabled={busy} onclick={() => reveal(k)}>Reveal</button>
                <button type="button" class="acct-link" disabled={busy} onclick={() => change(k)}>Change</button>
                <button type="button" class="acct-link" disabled={busy} onclick={() => remove(k)}>Remove</button>
              </span>
            {/if}
          </div>
        {/each}
      </div>
      <p class="vars-note">Stays on this Mac. Never included when you publish or share this extension.</p>
    {/if}

    {#if error}<p class="vars-error" role="alert">{error}</p>{/if}

    <footer class="vars-foot">
      <button class="btn ghost" type="button" onclick={onclose}>Cancel</button>
      <button class="btn pri" type="submit" disabled={!dirty || busy}>{busy ? 'Saving…' : 'Save'}</button>
    </footer>
  </form>
</div>
