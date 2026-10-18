<script lang="ts">
  import { onMount } from 'svelte';
  import type { EngineProbeResult } from '../../../shared/engine-ipc';
  import type { MediaFacts } from '../../../shared/engine-protocol';
  import { fancySelect } from '../controls/select/enhance';
  import {
    AUTOMATIC, LABELS, SECTIONS, automaticLabel, choicesFor, draftOf, originText, settingOf, storedOf,
    type Draft, type InterpretMember
  } from './interpret';

  /* Interpret Footage: one row per setting the file has, each showing its
     value and where that value comes from (the file, a default rule, or an
     override). Choosing a value re-reads the file with the draft, so the
     origins, the display size, and the duration below stay true; Apply
     writes the asset's `interpret` member as one Undo step. */
  let { name, initial, probe, onapply, onclose }: {
    name: string;
    initial: unknown;
    probe: (interpret: Record<string, unknown> | null) => Promise<EngineProbeResult>;
    onapply: (draft: Draft, facts: MediaFacts) => void;
    onclose: () => void;
  } = $props();

  const uid = `interp-${Math.random().toString(36).slice(2, 8)}`;
  let draft = $state<Draft>({});
  let facts = $state<MediaFacts | null>(null);
  let unavailable = $state('');
  let loading = $state(true);
  let seq = 0;

  const sections = $derived(
    SECTIONS.map((section) => ({
      title: section.title,
      members: section.members.filter((member) => settingOf(facts, member))
    })).filter((section) => section.members.length)
  );
  const changed = $derived(JSON.stringify(storedOf(draft)) !== JSON.stringify(storedOf(draftOf(initial))));

  async function reload(): Promise<void> {
    const mine = ++seq;
    loading = true;
    let result: EngineProbeResult;
    try {
      result = await probe(storedOf(draft) as Record<string, unknown> | null);
    } catch (cause) {
      result = { status: 'unavailable', reason: cause instanceof Error ? cause.message : 'The file could not be read.' };
    }
    if (mine !== seq) return;
    loading = false;
    if (result.status === 'facts') {
      facts = result.facts;
      unavailable = '';
    } else {
      unavailable = result.reason;
    }
  }

  function choose(member: InterpretMember, value: string): void {
    draft = { ...draft, [member]: value === AUTOMATIC ? null : value };
    void reload();
  }

  function toggleCleanAperture(): void {
    const setting = settingOf(facts, 'cleanAperture');
    const on = setting?.value === 'true';
    choose('cleanAperture', String(!on));
  }

  function setMatte(value: string): void {
    draft = { ...draft, alphaMatte: value };
  }

  function apply(): void {
    if (!facts) return;
    onapply(draft, facts);
  }

  onMount(() => {
    draft = draftOf(initial);
    void reload();
  });
</script>

<div class="interp-sheet" tabindex="-1" data-autofocus>
  <header class="acct-head">
    <h2>Interpret Footage</h2>
    <p><b>{name}</b></p>
  </header>

  <div class="sg-column interp-form">
    {#if unavailable && !facts}
      <p class="interp-note" role="status">{unavailable}</p>
    {:else if !facts}
      <p class="interp-note" role="status">Reading the file…</p>
    {:else}
      {#each sections as section (section.title)}
        <section class="sg-section interp-section">
          <h3 class="sg-section-title">{section.title}</h3>
          <div class="sg-group">
            {#each section.members as member (member)}
              {@const setting = settingOf(facts, member)}
              {@const overridden = setting?.origin.kind === 'override'}
              <div class="settings-row interp-row">
                <label class="settings-copy" for={`${uid}-${member}`}>
                  <b>{LABELS[member]}</b>
                  <span class:interp-override={overridden}>{originText(setting?.origin, setting?.automatic, member)}</span>
                </label>
                {#if member === 'cleanAperture'}
                  <button
                    id={`${uid}-${member}`}
                    class="toggle"
                    class:on={setting?.value === 'true'}
                    type="button"
                    aria-pressed={setting?.value === 'true'}
                    aria-label="Crop to the clean aperture"
                    onclick={toggleCleanAperture}
                  ><i aria-hidden="true"></i></button>
                {:else}
                  {@const value = draft[member] ?? AUTOMATIC}
                  <select
                    id={`${uid}-${member}`}
                    class="settings-select"
                    aria-label={LABELS[member]}
                    {value}
                    use:fancySelect={value}
                    onchange={(event) => choose(member, (event.currentTarget as HTMLSelectElement).value)}
                  >
                    <option value={AUTOMATIC}>{automaticLabel(member, setting)}</option>
                    {#each choicesFor(member, setting) as [choice, label] (choice)}
                      <option value={choice}>{label}</option>
                    {/each}
                  </select>
                {/if}
              </div>
              {#if member === 'alpha' && draft.alpha === 'premultipliedWithColor'}
                <div class="settings-row interp-row">
                  <label class="settings-copy" for={`${uid}-matte`}>
                    <b>Matte color</b>
                    <span>The color the source was premultiplied against, as #RRGGBB</span>
                  </label>
                  <input
                    id={`${uid}-matte`}
                    class="settings-input interp-matte"
                    type="text"
                    spellcheck="false"
                    autocomplete="off"
                    maxlength="7"
                    value={draft.alphaMatte ?? '#000000'}
                    onchange={(event) => setMatte((event.currentTarget as HTMLInputElement).value.trim())}
                  />
                </div>
              {/if}
            {/each}
          </div>
        </section>
      {/each}

      <section class="sg-section interp-section">
        <h3 class="sg-section-title">File</h3>
        <dl class="interp-facts">
          {#if facts.displaySize}
            <div><dt>Shown at</dt><dd>{facts.displaySize[0]} × {facts.displaySize[1]}</dd></div>
          {/if}
          {#each facts.facts as line (line.label)}
            <div><dt>{line.label}</dt><dd>{line.value}</dd></div>
          {/each}
        </dl>
      </section>

      {#if facts.issues.length}
        <p class="interp-error" role="alert">{facts.issues.join(' · ')}</p>
      {/if}
      {#if unavailable}
        <p class="interp-error" role="alert">{unavailable}</p>
      {/if}
    {/if}

    <footer class="interp-foot">
      <button class="btn ghost" type="button" onclick={onclose}>Cancel</button>
      <button class="btn pri" type="button" disabled={!facts || loading || !changed} onclick={apply}>Apply</button>
    </footer>
  </div>
</div>
