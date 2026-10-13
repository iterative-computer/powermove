<script lang="ts">
  import { onMount } from 'svelte';
  import type { TranscriptionBridge, TranscriptionModelInfo, TranscriptionStatus } from '../../../shared/transcription';
  import ModelMeters from './ModelMeters.svelte';
  import { captionGap, captionGapLine, downloadLine, modelFacts, modelFor, sheetModels, type ModelNeeds } from './format';

  /* The on-demand sheet: one line on why it appeared, the curated models to
     choose from (Settings › Transcription has the rest), and the download's
     progress. It closes itself once a model that fits is ready; hiding it
     mid-download leaves the download running. */
  let { host, initial, reason: initialReason, needs = {}, onfinish, onbrowse }: {
    host: TranscriptionBridge;
    initial: TranscriptionStatus | null;
    reason: string;
    /** Captions pass { wordTimestamps: true }: only word-timed models fit. */
    needs?: ModelNeeds;
    onfinish: (ready: boolean) => void;
    onbrowse?: () => void;
  } = $props();

  let status = $state<TranscriptionStatus | null>(null);
  let reason = $state('');
  let chosen = $state<string | null>(null);
  let busy = $state(false);
  let error = $state('');
  const uid = `tr-${Math.random().toString(36).slice(2, 8)}`;

  $effect.pre(() => {
    if (status === null && initial) status = initial;
    if (!reason) reason = initialReason;
  });

  export function setReason(next: string): void {
    if (next) reason = next;
  }

  const all = $derived(status?.models ?? []);
  const models = $derived(sheetModels(status, needs));
  const downloading = $derived(all.find((model) => model.state === 'downloading') ?? null);
  const selected = $derived(models.find((model) => model.id === chosen)
    ?? models.find((model) => model.id === downloading?.id) ?? models.find((model) => model.recommended) ?? models[0] ?? null);
  /* Captions asked, and the model in use cannot time words: say why it (and any word-timed model on this Mac) is not enough. */
  const gap = $derived(needs.wordTimestamps ? captionGap(status) : null);
  const selectedError = $derived(selected?.state === 'error' ? selected.error ?? 'The download failed. Try again.' : '');

  const message = (cause: unknown, fallback: string): string =>
    cause instanceof Error && cause.message ? cause.message.replace(/^Error invoking remote method '[^']+': (?:Error: )?/, '') : fallback;

  function apply(next: TranscriptionStatus): void {
    status = next;
    if (modelFor(next, needs)) onfinish(true);
  }

  async function download(): Promise<void> {
    if (!selected || busy) return;
    busy = true;
    error = '';
    try {
      apply(await host.download(selected.id));
    } catch (cause) {
      error = message(cause, 'Unable to start the download. Try again.');
    } finally {
      busy = false;
    }
  }

  async function cancel(model: TranscriptionModelInfo): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      apply(await host.cancelDownload(model.id));
    } catch (cause) {
      error = message(cause, 'Unable to cancel the download.');
    } finally {
      busy = false;
    }
  }

  function label(model: TranscriptionModelInfo): string {
    return model.state === 'error' ? 'Try Again' : (model.downloadedBytes ?? 0) > 0 ? 'Resume Download' : 'Download';
  }

  onMount(() => {
    const off = host.onStatus(apply);
    void host.status().then(apply).catch(() => undefined);
    return off;
  });
</script>

<div class="tr-sheet" tabindex="-1" data-autofocus>
  <header class="acct-head">
    <h2>Download a speech model</h2>
    <p>{reason}</p>
    {#if gap}<p class="tr-why">{captionGapLine(gap)}</p>{/if}
  </header>

  <div class="sg-column tr-form">
    <div class="sg-group" role="radiogroup" aria-label="Speech model">
      {#each models as model (model.id)}
        {@const checked = selected?.id === model.id}
        <label class="settings-row tr-model" class:is-checked={checked}>
          <input
            type="radio"
            name={`${uid}-model`}
            value={model.id}
            checked={checked}
            disabled={!!downloading && downloading.id !== model.id}
            onchange={() => { chosen = model.id; }}
          />
          <span class="tr-radio" aria-hidden="true"></span>
          <span class="settings-copy">
            <b>{model.name}{#if model.recommended}<span class="tr-tag">Recommended</span>{/if}</b>
            <span>{model.description}</span>
            <span class="tr-size">{modelFacts(model)}</span>
          </span>
          <ModelMeters speed={model.speed} accuracy={model.accuracy} />
        </label>
      {/each}
    </div>

    {#if downloading}
      <div class="tr-progress" role="status" aria-live="polite">
        <div class="tr-progress-line">
          <span>Downloading {downloading.name}</span>
          <span class="tr-progress-count">{downloadLine(downloading)}</span>
        </div>
        <div class="tr-bar" role="progressbar" aria-label={`Downloading ${downloading.name}`} aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round((downloading.progress ?? 0) * 100)}>
          <i style:transform={`scaleX(${downloading.progress ?? 0})`}></i>
        </div>
      </div>
    {/if}

    {#if error || selectedError}<p class="tr-error" role="alert">{error || selectedError}</p>{/if}
    <p class="tr-note">Runs on this Mac. Your audio is never uploaded.</p>

    <footer class="tr-foot">
      {#if onbrowse}<button class="btn ghost tr-browse" type="button" onclick={onbrowse}>Show All Models</button>{/if}
      {#if downloading}
        <button class="btn ghost" type="button" disabled={busy} onclick={() => cancel(downloading)}>Cancel Download</button>
        <button class="btn pri" type="button" onclick={() => onfinish(false)}>Hide</button>
      {:else}
        <button class="btn ghost" type="button" onclick={() => onfinish(false)}>Not Now</button>
        <button class="btn pri" type="button" disabled={busy || !selected} onclick={download}>{selected ? label(selected) : 'Download'}</button>
      {/if}
    </footer>
  </div>
</div>
