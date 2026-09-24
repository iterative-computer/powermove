<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import type { StoreBridge } from '../../../shared/store-ipc';
  import {
    PUBLISH_LICENCES,
    PUBLISH_LIMITS,
    PUBLISH_TERMS,
    findingKey,
    type PublishPlanDto,
    type PublishProgress,
    type StorePublishResult
  } from '../../../shared/publish';
  import { KINDS, KIND_PLURAL, publishErrorText } from './data';
  import {
    LICENCE_LABEL,
    canPublish,
    checkIconBytes,
    draftFor,
    findingLabel,
    formFor,
    isLicence,
    problemsOf,
    progressText,
    type PublishDraft
  } from './publish-form';
  import SandboxCheckStatus from './SandboxCheckStatus.svelte';
  import { SANDBOX_UNAVAILABLE, type SandboxCheckReport, type SandboxCheckState } from './sandbox-check';

  let { plan, bridge, check, onclose, onpublished }: {
    plan: PublishPlanDto;
    bridge: StoreBridge;
    /** Runs the sandbox check for this extension as it is built now (kernel/sandbox-check.ts). */
    check: () => Promise<SandboxCheckReport>;
    onclose: () => void;
    onpublished: (result: Extract<StorePublishResult, { published: true }>) => void;
  } = $props();

  /* One column: what is being published, the version and its notes, the
     listing on a first publish, anything the scanner found, then the terms
     and the button. The button opens main's native confirmation, which names
     the exact snapshot; this sheet never sees the bytes. */
  const uid = `publish-${Math.random().toString(36).slice(2, 8)}`;
  const initial = untrack(() => plan);
  let draft = $state<PublishDraft>(draftFor(initial));
  let touched = $state<Record<string, boolean>>({});
  let attempted = $state(false);
  let busy = $state(false);
  let progress = $state<PublishProgress | null>(null);
  let error = $state<string | null>(null);
  let published = $state<Extract<StorePublishResult, { published: true }> | null>(null);
  let iconUrl = $state<string | null>(null);
  let iconError = $state<string | null>(null);
  let iconInput = $state<HTMLInputElement | null>(null);

  /* Required step: the check runs as the sheet opens, and only a clean (or
     full-access, skipped) result lets Publish through. */
  let sandbox = $state<SandboxCheckState>({ status: 'running' });
  let checkRun = 0;
  async function runCheck(): Promise<void> {
    const run = ++checkRun;
    sandbox = { status: 'running' };
    let next: SandboxCheckState;
    try {
      next = { status: 'done', report: await check() };
    } catch {
      next = { status: 'error', message: SANDBOX_UNAVAILABLE };
    }
    if (run === checkRun) sandbox = next;
  }
  $effect(() => { untrack(() => void runCheck()); });

  const problems = $derived(problemsOf(draft, plan));
  const sandboxPassed = $derived(sandbox.status === 'done' && sandbox.report.ok);
  const ready = $derived(canPublish(problems) && sandboxPassed);
  const title = $derived(plan.isFork && plan.firstPublish ? `Publish your version of ${plan.manifest.name}` : plan.firstPublish ? `Publish ${plan.manifest.name}` : `Publish an update to ${plan.listing.name}`);
  const show = (field: string): boolean => attempted || !!touched[field];

  $effect(() => bridge.onPublishProgress((next) => {
    if (next.localId === plan.localId) progress = next;
  }));
  onDestroy(() => {
    if (iconUrl) URL.revokeObjectURL(iconUrl);
  });

  async function chooseIcon(event: Event): Promise<void> {
    const input = event.currentTarget instanceof HTMLInputElement ? event.currentTarget : null;
    const file = input?.files?.[0];
    if (!input || !file) return;
    input.value = '';
    iconError = null;
    const checked = checkIconBytes(new Uint8Array(await file.arrayBuffer()));
    if (!checked.ok) {
      iconError = checked.error;
      return;
    }
    if (iconUrl) URL.revokeObjectURL(iconUrl);
    iconUrl = URL.createObjectURL(file);
    draft.iconPng = checked.base64;
  }

  function removeIcon(): void {
    if (iconUrl) URL.revokeObjectURL(iconUrl);
    iconUrl = null;
    iconError = null;
    draft.iconPng = null;
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    attempted = true;
    if (busy || !ready) return;
    busy = true;
    error = null;
    progress = null;
    try {
      const result = await bridge.publish({ localId: plan.localId, form: formFor(draft, plan) });
      if (!result.ok) {
        error = publishErrorText(result.error);
        return;
      }
      if (result.value.published) {
        published = result.value;
        onpublished(result.value);
      }
    } catch {
      error = 'Powermove couldn’t publish from this window. Try again.';
    } finally {
      busy = false;
      progress = null;
    }
  }

  function setLicence(event: Event): void {
    const value = event.currentTarget instanceof HTMLSelectElement ? event.currentTarget.value : '';
    if (isLicence(value)) draft.licence = value;
  }

  function setCategory(event: Event): void {
    const value = event.currentTarget instanceof HTMLSelectElement ? event.currentTarget.value : '';
    const kind = KINDS.find((candidate) => candidate === value);
    if (kind) draft.category = kind;
  }

  function setVisibility(event: Event): void {
    const value = event.currentTarget instanceof HTMLSelectElement ? event.currentTarget.value : '';
    if (value === 'public' || value === 'unlisted') draft.visibility = value;
  }
</script>

<!-- svelte-ignore a11y_autofocus -->
<div class="pub-sheet" tabindex="-1" autofocus>
  {#if published}
    <header class="acct-head">
      <h2>Published {published.coordinate} {published.version}</h2>
      <p>{plan.firstPublish
        ? draft.visibility === 'public' ? 'It’s on the store now, in Browse and search.' : 'It’s on the store now. Anyone with its link can install it.'
        : 'It’s on the store now. People who have it get the update the next time Powermove checks.'}</p>
    </header>
    <footer class="pub-foot">
      <span></span>
      <button class="btn pri" type="button" onclick={onclose}>Done</button>
    </footer>
  {:else}
    <header class="acct-head">
      <h2>{title}</h2>
      {#if plan.isFork && plan.origin}
        <p>Forked from <b>{plan.origin.coordinate}</b> {plan.origin.version}. It goes on the store under your name, linked to the original.</p>
      {/if}
    </header>

    <form class="pub-form" novalidate onsubmit={submit}>
      <div class="sg-group pub-group">
        <div class="settings-row pub-row">
          <span class="settings-copy"><b>Extension</b></span>
          <span class="pub-fixed">{plan.coordinate}</span>
        </div>
        <div class="settings-row pub-row">
          <label class="settings-copy" for={`${uid}-version`}>
            <b>Version</b>
            {#if show('version') && problems.version}
              <span class="pub-problem" id={`${uid}-version-problem`}>{problems.version}</span>
            {:else if plan.lastVersion}
              <span>Last published {plan.lastVersion}</span>
            {/if}
          </label>
          <input
            id={`${uid}-version`}
            class="pub-input is-short"
            inputmode="decimal"
            autocomplete="off"
            spellcheck="false"
            maxlength="20"
            disabled={busy}
            aria-invalid={show('version') && !!problems.version}
            aria-describedby={show('version') && problems.version ? `${uid}-version-problem` : undefined}
            bind:value={draft.version}
            onblur={() => (touched = { ...touched, version: true })}
          />
        </div>
        <div class="settings-row pub-row is-stacked">
          <label class="settings-copy" for={`${uid}-notes`}><b>What’s new</b></label>
          <textarea
            id={`${uid}-notes`}
            class="pub-input pub-notes"
            rows="3"
            maxlength={PUBLISH_LIMITS.notesChars}
            placeholder={plan.firstPublish ? 'Optional' : 'What changed in this version'}
            disabled={busy}
            bind:value={draft.notes}
          ></textarea>
        </div>
      </div>

      {#if plan.firstPublish}
        <h3 class="pub-title">On the store</h3>
        <div class="sg-group pub-group">
          <div class="settings-row pub-row">
            <label class="settings-copy" for={`${uid}-name`}>
              <b>Name</b>
              {#if show('name') && problems.name}<span class="pub-problem" id={`${uid}-name-problem`}>{problems.name}</span>{/if}
            </label>
            <input
              id={`${uid}-name`}
              class="pub-input"
              autocomplete="off"
              maxlength={PUBLISH_LIMITS.nameChars}
              disabled={busy}
              aria-invalid={show('name') && !!problems.name}
              aria-describedby={show('name') && problems.name ? `${uid}-name-problem` : undefined}
              bind:value={draft.name}
              onblur={() => (touched = { ...touched, name: true })}
            />
          </div>
          <div class="settings-row pub-row">
            <label class="settings-copy" for={`${uid}-tagline`}>
              <b>Tagline</b>
              {#if problems.tagline}<span class="pub-problem">{problems.tagline}</span>{:else}<span>One line under the name</span>{/if}
            </label>
            <input id={`${uid}-tagline`} class="pub-input" autocomplete="off" maxlength={PUBLISH_LIMITS.taglineChars} disabled={busy} bind:value={draft.tagline} />
          </div>
          <div class="settings-row pub-row">
            <label class="settings-copy" for={`${uid}-category`}><b>Kind</b></label>
            <select id={`${uid}-category`} class="pub-select" disabled={busy} value={draft.category} onchange={setCategory}>
              {#each KINDS as kind (kind)}<option value={kind}>{KIND_PLURAL[kind]}</option>{/each}
            </select>
          </div>
          <div class="settings-row pub-row">
            <label class="settings-copy" for={`${uid}-licence`}><b>Licence</b></label>
            <select id={`${uid}-licence`} class="pub-select" disabled={busy} value={draft.licence} onchange={setLicence}>
              {#each PUBLISH_LICENCES as licence (licence)}<option value={licence}>{LICENCE_LABEL[licence]}</option>{/each}
            </select>
          </div>
          <div class="settings-row pub-row">
            <label class="settings-copy" for={`${uid}-visibility`}>
              <b>Visibility</b>
              <span>{draft.visibility === 'public' ? 'Shown in Browse and search' : 'Only people with the link can find it'}</span>
            </label>
            <select id={`${uid}-visibility`} class="pub-select" disabled={busy} value={draft.visibility} onchange={setVisibility}>
              <option value="public">Public</option>
              <option value="unlisted">Unlisted</option>
            </select>
          </div>
          <div class="settings-row pub-row">
            <span class="settings-copy">
              <b>Icon</b>
              {#if iconError}<span class="pub-problem" role="alert">{iconError}</span>{:else}<span>Square PNG, up to 256 KB</span>{/if}
            </span>
            <span class="pub-icon">
              {#if iconUrl}<img class="pub-icon-preview" src={iconUrl} alt="Icon preview" />{/if}
              <input bind:this={iconInput} class="pub-file" type="file" accept="image/png" tabindex="-1" aria-hidden="true" onchange={chooseIcon} />
              {#if iconUrl}
                <button class="acct-link" type="button" disabled={busy} onclick={removeIcon}>Remove</button>
              {/if}
              <button class="btn" type="button" disabled={busy} onclick={() => iconInput?.click()}>{iconUrl ? 'Change…' : 'Choose…'}</button>
            </span>
          </div>
        </div>
      {/if}

      <h3 class="pub-title">Sandbox check</h3>
      <SandboxCheckStatus state={sandbox} onretry={busy ? undefined : () => void runCheck()} />

      {#if plan.permissionFindings.length}
        <h3 class="pub-title">Permissions to declare</h3>
        <div class="sg-group pub-group">
          {#each plan.permissionFindings as finding (`${finding.path}:${finding.line}:${finding.needs}`)}
            <div class="settings-row pub-row">
              <span class="settings-copy"><span class="pub-problem">{finding.text}</span></span>
            </div>
          {/each}
        </div>
      {/if}

      {#if plan.blockedFindings.length || plan.waivableFindings.length}
        <h3 class="pub-title">Possible secrets</h3>
        <div class="sg-group pub-group">
          {#each plan.blockedFindings as finding (`${findingKey(finding)}:${finding.kind}`)}
            <div class="settings-row pub-row">
              <span class="settings-copy">
                <b class="pub-path">{finding.path}<i>line {finding.line}</i></b>
                <span class="pub-problem">{findingLabel(finding.kind)}. Remove this before publishing.</span>
              </span>
            </div>
          {/each}
          {#each plan.waivableFindings as finding (findingKey(finding))}
            {@const key = findingKey(finding)}
            <div class="settings-row pub-row">
              <label class="settings-copy" for={`${uid}-why-${key}`}>
                <b class="pub-path">{finding.path}<i>line {finding.line}</i></b>
                {#if show(`why:${key}`) && problems.reasons[key]}
                  <span class="pub-problem">{problems.reasons[key]}</span>
                {:else}
                  <span>{findingLabel(finding.kind)}. If it isn’t a secret, say why.</span>
                {/if}
              </label>
              <input
                id={`${uid}-why-${key}`}
                class="pub-input"
                placeholder="Why it’s safe"
                autocomplete="off"
                maxlength={PUBLISH_LIMITS.waiverReasonMax}
                disabled={busy}
                aria-invalid={show(`why:${key}`) && !!problems.reasons[key]}
                bind:value={draft.reasons[key]}
                onblur={() => (touched = { ...touched, [`why:${key}`]: true })}
              />
            </div>
          {/each}
        </div>
      {/if}

      <p class="pub-terms">{PUBLISH_TERMS}</p>
      {#if error}<p class="pub-error" role="alert">{error}</p>{/if}

      <footer class="pub-foot">
        <span class="pub-status" aria-live="polite">{busy ? progressText(progress) : ''}</span>
        <button class="btn ghost" type="button" disabled={busy} onclick={onclose}>Cancel</button>
        <button class="btn pri" type="submit" disabled={busy || problems.blocked || !sandboxPassed || (attempted && !ready)}>Publish…</button>
      </footer>
    </form>
  {/if}
</div>
