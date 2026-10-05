<script lang="ts">
  import { onMount } from 'svelte';
  import type { TranscriptionModelInfo, TranscriptionStatus } from '../../../shared/transcription';
  import ModelMeters from './ModelMeters.svelte';
  import { transcriptionBridge } from './host';
  import { downloadLine, formatBytes, languageOptions, percent } from './format';

  /* Settings › Transcription: the models on this Mac and the one in use,
     the spoken language, and where the models live. */
  let { PM }: { PM: Record<string, any> } = $props();

  const host = transcriptionBridge();
  let status = $state<TranscriptionStatus | null>(null);
  let busy = $state<string | null>(null);
  let error = $state('');

  const models = $derived(status?.models ?? []);
  const active = $derived(models.find((model) => model.id === status?.activeModelId) ?? null);
  const languages = $derived(languageOptions(active));
  const language = $derived(status?.language ?? 'auto');
  const unavailable = $derived(!host || status?.available === false);

  const message = (cause: unknown, fallback: string): string =>
    cause instanceof Error && cause.message ? cause.message.replace(/^Error invoking remote method '[^']+': (?:Error: )?/, '') : fallback;

  async function run(key: string, action: () => Promise<TranscriptionStatus | void>, fallback: string): Promise<void> {
    if (busy) return;
    busy = key;
    error = '';
    try {
      const next = await action();
      if (next) status = next;
    } catch (cause) {
      error = message(cause, fallback);
    } finally {
      busy = null;
    }
  }

  async function remove(model: TranscriptionModelInfo): Promise<void> {
    if (!host) return;
    const confirmed = await PM.confirm?.({
      message: `Delete ${model.name}?`,
      detail: `This frees ${formatBytes(model.size)}. You can download it again at any time.`,
      confirmLabel: 'Delete',
      destructive: true
    });
    if (!confirmed) return;
    await run(model.id, () => host.remove(model.id), `Unable to delete ${model.name}.`);
  }

  function facts(model: TranscriptionModelInfo): string {
    const reach = model.languages === 'en' ? 'English' : `${model.languageCodes?.length ?? 'Many'} languages`;
    return `${formatBytes(model.size)} · ${reach}`;
  }

  onMount(() => {
    if (!host) return;
    const off = host.onStatus((next) => { status = next; });
    void host.status().then((next) => { status = next; }).catch(() => { error = 'Unable to read the transcription models.'; });
    return off;
  });
</script>

{#if unavailable}
  <section class="sg-section">
    <h3 class="sg-section-title">Models</h3>
    <div class="sg-group">
      <div class="settings-row">
        <div class="settings-copy">
          <b>Not available here</b>
          <span>On-device transcription runs in the Powermove app on a Mac.</span>
        </div>
      </div>
    </div>
  </section>
{:else}
  <section class="sg-section">
    <h3 class="sg-section-title">Models</h3>
    <div class="sg-group tr-settings-models">
      {#each models as model (model.id)}
        {@const inUse = model.id === status?.activeModelId}
        <div class="settings-row tr-settings-row" data-model={model.id}>
          <div class="settings-copy">
            <b>
              {model.name}
              {#if inUse}<span class="tr-tag">In use</span>{:else if model.recommended}<span class="tr-tag">Recommended</span>{/if}
            </b>
            <span>{model.description}</span>
            <span class="tr-size">{facts(model)}</span>
            {#if model.state === 'downloading'}
              <span class="tr-settings-progress">
                <span class="tr-bar" role="progressbar" aria-label={`Downloading ${model.name}`} aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round((model.progress ?? 0) * 100)}>
                  <i style:transform={`scaleX(${model.progress ?? 0})`}></i>
                </span>
                <span class="tr-progress-count">{downloadLine(model)} · {percent(model.progress)}</span>
              </span>
            {:else if model.state === 'error'}
              <span class="tr-row-error">{model.error ?? 'The download failed.'}</span>
            {/if}
          </div>
          <ModelMeters speed={model.speed} accuracy={model.accuracy} />
          <div class="tr-settings-actions">
            {#if model.state === 'downloading'}
              <button class="btn" type="button" disabled={busy === model.id} onclick={() => run(model.id, () => host!.cancelDownload(model.id), 'Unable to cancel the download.')}>Cancel</button>
            {:else if model.state === 'ready'}
              {#if !inUse}
                <button class="btn" type="button" disabled={!!busy} onclick={() => run(model.id, () => host!.setActive(model.id), `Unable to use ${model.name}.`)}>Use</button>
              {/if}
              <button class="btn" type="button" disabled={!!busy} onclick={() => remove(model)}>Delete…</button>
            {:else}
              <button class="btn" type="button" disabled={!!busy} onclick={() => run(model.id, () => host!.download(model.id), 'Unable to start the download.')}>
                {model.state === 'error' ? 'Try Again' : (model.downloadedBytes ?? 0) > 0 ? 'Resume' : 'Download'}
              </button>
            {/if}
          </div>
        </div>
      {/each}
    </div>
    {#if error}<p class="settings-note tr-row-error" role="alert">{error}</p>{/if}
    <p class="settings-note">Transcription runs on this Mac and your audio is never uploaded. Parakeet models are by NVIDIA, under CC BY 4.0.</p>
  </section>

  <section class="sg-section">
    <h3 class="sg-section-title">Language</h3>
    <div class="sg-group">
      <div class="settings-row">
        <div class="settings-copy">
          <b>Spoken language</b>
          <span>{active?.languages === 'en'
            ? `${active.name} transcribes English.`
            : 'Detected automatically. Choose one to label transcripts for captions and subtitles.'}</span>
        </div>
        <select class="settings-select" aria-label="Spoken language" value={active?.languages === 'en' ? 'en' : language}
          disabled={!active || active.languages === 'en' || !!busy}
          onchange={(event) => { const value = event.currentTarget.value; void run('language', () => host!.setLanguage(value), 'Unable to change the language.'); }}>
          {#if active?.languages === 'en'}
            <option value="en">English</option>
          {:else}
            <option value="auto">Detect automatically</option>
            {#each languages as option (option.code)}<option value={option.code}>{option.name}</option>{/each}
          {/if}
        </select>
      </div>
    </div>
  </section>

  <section class="sg-section">
    <h3 class="sg-section-title">Storage</h3>
    <div class="sg-group">
      <div class="settings-row">
        <div class="settings-copy">
          <b>Downloaded models</b>
          <span>{(status?.storageBytes ?? 0) > 0 ? `${formatBytes(status?.storageBytes ?? 0)} in Powermove’s data folder.` : 'No models on this Mac yet.'}</span>
        </div>
        <button class="btn" type="button" onclick={() => void host!.reveal().catch((cause: unknown) => { error = message(cause, 'Unable to show the folder.'); })}>Show in Finder</button>
      </div>
    </div>
  </section>
{/if}
