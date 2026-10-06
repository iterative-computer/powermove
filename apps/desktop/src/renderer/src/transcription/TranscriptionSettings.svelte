<script lang="ts">
  import { onMount } from 'svelte';
  import type { TranscriptionModelInfo, TranscriptionStatus } from '../../../shared/transcription';
  import ModelMeters from './ModelMeters.svelte';
  import { transcriptionBridge } from './host';
  import { captionGap, captionSuggestion, catalogLanguages, downloadLine, formatBytes, languageName, languageOptions, modelFacts, modelFor, modelsFor, percent } from './format';

  /* Settings › Transcription: the models on this Mac (the one in use first
     among them), the ones to download, filterable by language once the list
     is long, the spoken language, and where the models live. */
  let { PM }: { PM: Record<string, any> } = $props();

  const host = transcriptionBridge();
  let status = $state<TranscriptionStatus | null>(null);
  let busy = $state<string | null>(null);
  let error = $state('');

  const models = $derived(status?.models ?? []);
  const active = $derived(models.find((model) => model.id === status?.activeModelId) ?? null);
  /* A model that cannot detect hears English on 'auto', so that option is English and 'en' is not listed again. */
  const hearsEnglish = $derived(active?.detectsLanguage === false);
  const languages = $derived(languageOptions(active).filter((option) => !hearsEnglish || option.code !== 'en'));
  const language = $derived(hearsEnglish && status?.language === 'en' ? 'auto' : status?.language ?? 'auto');
  const unavailable = $derived(!host || status?.available === false);

  /* On this Mac; a download in flight stays where it was started until it lands. */
  const downloaded = $derived(models.filter((model) => model.state === 'ready'));
  const available = $derived(models.filter((model) => model.state !== 'ready'));
  /** A language filter only once the list is long enough to need one. */
  const filterable = $derived(models.length > 5);
  let filter = $state('all');
  /* Every model's languages, so the choice survives downloading the last model for one. */
  const filterLanguages = $derived(catalogLanguages(models));
  const shown = $derived(filterable ? modelsFor(available, filter) : available);
  /* Captions need words timed; say so only when the model in use cannot. */
  const captionsNote = $derived.by(() => {
    if (!active || active.state !== 'ready' || active.wordTimestamps) return '';
    const timed = modelFor(status, { wordTimestamps: true });
    if (timed) return `${active.name} doesn’t time each word, so captions use ${timed.name}.`;
    const gap = captionGap(status);
    const suggestion = captionSuggestion(status);
    const such = suggestion ? ` such as ${suggestion.name}` : '';
    if (!gap?.unfit) return `${active.name} doesn’t time each word, so captions need a model that does${such ? `,${such}` : ''}.`;
    /* A word-timed model is here but cannot hear this speech. */
    return gap.language
      ? `${active.name} doesn’t time each word and ${gap.unfit.name} doesn’t transcribe ${languageName(gap.language)}, so captions need a model${such}.`
      : `${active.name} doesn’t time each word and ${gap.unfit.name} transcribes only English, so captions need a multilingual model${such}, or English as the spoken language.`;
  });

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

  onMount(() => {
    if (!host) return;
    const off = host.onStatus((next) => { status = next; });
    void host.status().then((next) => { status = next; }).catch(() => { error = 'Unable to read the transcription models.'; });
    return off;
  });
</script>

{#snippet footnote()}
  {#if error}<p class="settings-note tr-row-error" role="alert">{error}</p>{/if}
  <p class="settings-note">Transcription runs on this Mac and your audio is never uploaded. Models are by NVIDIA, OpenAI and Cohere under their own licenses; Parakeet and Canary are CC BY 4.0.</p>
{/snippet}

{#snippet row(model: TranscriptionModelInfo)}
  {@const inUse = model.id === status?.activeModelId}
  <div class="settings-row tr-settings-row" data-model={model.id}>
    <div class="settings-copy">
      <b>
        {model.name}
        {#if inUse}<span class="tr-tag">In use</span>{:else if model.recommended && model.state !== 'ready'}<span class="tr-tag">Recommended</span>{/if}
      </b>
      <span>{model.description}</span>
      <span class="tr-size">{modelFacts(model)}</span>
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
{/snippet}

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
  {#if downloaded.length}
    <section class="sg-section">
      <h3 class="sg-section-title">Downloaded</h3>
      <div class="sg-group tr-settings-models">
        {#each downloaded as model (model.id)}{@render row(model)}{/each}
      </div>
      {#if captionsNote}<p class="settings-note">{captionsNote}</p>{/if}
      {#if !available.length}{@render footnote()}{/if}
    </section>
  {/if}

  {#if available.length}
    <section class="sg-section">
      <div class="tr-section-head">
        <h3 class="sg-section-title">{downloaded.length ? 'Available to Download' : 'Models'}</h3>
        {#if filterable}
          <select class="settings-select tr-filter" aria-label="Show models for" value={filter}
            onchange={(event) => { filter = event.currentTarget.value; }}>
            <option value="all">All Languages</option>
            {#each filterLanguages as option (option.code)}<option value={option.code}>{option.name}</option>{/each}
          </select>
        {/if}
      </div>
      {#if shown.length}
        <div class="sg-group tr-settings-models">
          {#each shown as model (model.id)}{@render row(model)}{/each}
        </div>
      {:else}
        <p class="settings-note tr-empty">Every model for {languageName(filter)} is downloaded.</p>
      {/if}
      {@render footnote()}
    </section>
  {/if}

  <section class="sg-section">
    <h3 class="sg-section-title">Language</h3>
    <div class="sg-group">
      <div class="settings-row">
        <div class="settings-copy">
          <b>Spoken language</b>
          <span>{active?.languages === 'en'
            ? `${active.name} transcribes English.`
            : active?.detectsLanguage === false
              ? `${active.name} can’t detect the language, so it hears English until you choose another.`
              : 'Detected automatically. Choose one to label transcripts for captions and subtitles.'}</span>
        </div>
        <select class="settings-select" aria-label="Spoken language" value={active?.languages === 'en' ? 'en' : language}
          disabled={!active || active.languages === 'en' || !!busy}
          onchange={(event) => { const value = event.currentTarget.value; void run('language', () => host!.setLanguage(value), 'Unable to change the language.'); }}>
          {#if active?.languages === 'en'}
            <option value="en">English</option>
          {:else}
            <option value="auto">{hearsEnglish ? 'English' : 'Detect automatically'}</option>
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
