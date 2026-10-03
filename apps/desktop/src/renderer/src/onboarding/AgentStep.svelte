<script lang="ts">
  import { AGENT_MODELS } from '../../../shared/agent-models';
  import type { OnboardingAgentChoice, OnboardingAgentProvider } from '../../../shared/creative-workspace';

  let { onback, oncontinue }: { onback: () => void; oncontinue: (agent: OnboardingAgentChoice) => void } = $props();

  /* The bundled lists are newest first; live discovery adds more in the editor. */
  const options: Array<{ id: OnboardingAgentProvider; name: string; detail: string; models: Array<{ id: string; label: string }> }> = [
    { id: 'chatgpt', name: 'ChatGPT', detail: 'Uses your ChatGPT plan through Codex.', models: AGENT_MODELS.chatgpt },
    { id: 'claude', name: 'Claude', detail: 'Uses your Claude plan through Claude Code.', models: AGENT_MODELS.claude.filter(model => !/\(latest\)$/.test(model.label)) },
    { id: 'compatible', name: 'API or local model', detail: 'Any OpenAI-compatible API, Ollama or LM Studio. Connect it in Settings.', models: [] }
  ];
  let provider = $state<OnboardingAgentProvider>('chatgpt');
  const models = $state<Record<OnboardingAgentProvider, string>>({
    chatgpt: options[0]!.models[0]!.id,
    claude: options[1]!.models[0]!.id,
    compatible: 'configured'
  });
</script>

<main class="agent-step" aria-labelledby="agent-title">
  <button class="back" type="button" aria-label="Back to timeline choice" onclick={onback}>
    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m14 6-6 6 6 6" /></svg>
  </button>
  <h1 id="agent-title" tabindex="-1">Pick your agent.</h1>
  <p class="intro">It builds what you ask for, right in the editor.<br />You can switch anytime from the agent’s composer.</p>
  <div class="choices" role="radiogroup" aria-labelledby="agent-title">
    {#each options as option}
      {@const selected = provider === option.id}
      <label class="choice" class:selected>
        <input type="radio" name="agent-provider" value={option.id} bind:group={provider} />
        <span class="check" aria-hidden="true">
          <svg viewBox="0 0 16 16"><path d="m4.5 8.25 2.25 2.25 4.75-5" /></svg>
        </span>
        <span class="text">
          <span class="name">{option.name}</span>
          <span class="detail">{option.detail}</span>
        </span>
        {#if option.models.length}
          <span class="model">
            <select aria-label={`${option.name} model`} bind:value={models[option.id]} onfocus={() => { provider = option.id; }}>
              {#each option.models as model}<option value={model.id}>{model.label}</option>{/each}
            </select>
            <svg class="chevron" viewBox="0 0 16 16" aria-hidden="true"><path d="m4.5 6.25 3.5 3.5 3.5-3.5" /></svg>
          </span>
        {/if}
      </label>
    {/each}
  </div>
  <div class="actions">
    <button class="primary" type="button" onclick={() => oncontinue({ provider, model: models[provider] })}>Continue</button>
  </div>
</main>

<style>
  .agent-step { position: relative; z-index: 1; display: flex; flex-direction: column; align-items: center; width: min(560px, calc(100vw - 96px)); padding-top: 48px; text-align: center; -webkit-app-region: no-drag; }
  /* Same staggered entrance as the other steps (welcome-enter in welcome.css). */
  .agent-step > * { animation: welcome-enter 800ms cubic-bezier(0.16, 1, 0.3, 1) calc(240ms + var(--i, 0) * 120ms) backwards; }
  .agent-step > h1 { --i: 1; }
  .agent-step > .intro { --i: 2; }
  .agent-step > .choices { --i: 3; }
  .agent-step > .actions { --i: 4; }
  /* Lines up with the back arrow on the timeline and suitcase steps. */
  .back { position: absolute; top: 0; left: calc(50% - 170px); width: 32px; height: 32px; display: grid; place-items: center; background: transparent; color: var(--tx-3); }
  .back:hover { background: var(--ink-1); color: var(--tx); }
  h1 { margin: 0 0 12px; font-size: clamp(30px, 4vw, 36px); line-height: 1.1; font-weight: var(--fw-regular); letter-spacing: -.035em; white-space: nowrap; outline: none; }
  .intro { margin: 0; color: var(--tx-2); font-size: var(--fs-lg); line-height: 1.5; }
  .choices { display: flex; flex-direction: column; gap: 10px; width: 100%; margin-top: 28px; }
  .choice { position: relative; display: flex; align-items: center; gap: 12px; min-height: 64px; box-sizing: border-box; padding: 12px 12px 12px 18px; border-radius: var(--r-md); background: var(--ink-1); text-align: left; cursor: default; transition: background var(--dur-1) var(--ease), box-shadow var(--dur-1) var(--ease); }
  .choice:hover { background: var(--ink-2); }
  .choice.selected { background: var(--bg-float); box-shadow: var(--shadow-raise); }
  .choice:has(input:focus-visible) { outline: 2px solid var(--accent); outline-offset: 3px; }
  input { position: absolute; opacity: 0; pointer-events: none; }
  /* Selected: accent disc with a check. Unselected: an empty disc in the same spot. */
  .check { flex: none; display: grid; place-items: center; width: 18px; height: 18px; border-radius: 50%; background: var(--ink-2); transition: background var(--dur-1) var(--ease); }
  .check svg { width: 14px; height: 14px; stroke: var(--on-accent); stroke-width: 1.75; stroke-linecap: round; stroke-linejoin: round; opacity: 0; }
  .selected .check { background: var(--accent); }
  .selected .check svg { opacity: 1; }
  .text { display: grid; gap: 2px; flex: 1; min-width: 0; }
  .name { color: var(--tx); font-size: var(--fs-lg); font-weight: var(--fw-medium); }
  .detail { color: var(--tx-3); font-size: var(--fs-sm); line-height: 1.4; }
  .model { position: relative; flex: none; width: 160px; }
  select { appearance: none; width: 100%; height: 32px; padding: 0 28px 0 10px; border: 0; border-radius: var(--r-sm); background: var(--ink-1); color: var(--tx); font: 500 var(--fs-sm) 'Geist', sans-serif; transition: background var(--dur-1) var(--ease), opacity var(--dur-1) var(--ease); }
  select:hover { background: var(--ink-2); }
  .choice:not(.selected) select { opacity: .6; }
  select:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .chevron { position: absolute; top: 50%; right: 8px; width: 14px; height: 14px; translate: 0 -50%; stroke: var(--tx-3); stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; pointer-events: none; }
  .actions { display: flex; align-items: center; gap: 8px; margin-top: 28px; }
  svg { fill: none; stroke: none; }
  .back svg { width: 20px; height: 20px; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  button { border: 0; border-radius: var(--r-sm); font: 500 var(--fs-sm) 'Geist', sans-serif; cursor: default; transition: background var(--dur-1) var(--ease); }
  .primary { min-height: 38px; padding: 0 18px; color: var(--on-accent); background: var(--accent); }
  .primary:hover { background: var(--accent-hover); }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 3px; }
  /* Short windows: tighter spacing so the whole step fits without scrolling. */
  @media (max-height: 600px) { .agent-step { width: min(480px, calc(100vw - 96px)); padding-top: 40px; } .choices, .actions { margin-top: 20px; } .choice { min-height: 56px; padding-block: 9px; } }
  @media (prefers-reduced-motion: reduce) { .agent-step > * { animation: none; } button, .choice, select { transition: none; } }
</style>
