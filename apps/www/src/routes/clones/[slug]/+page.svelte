<script lang="ts">
  import Header from '$lib/Header.svelte';
  import Footer from '$lib/Footer.svelte';
  import Seo from '$lib/Seo.svelte';
  import Short from '$lib/Short.svelte';
  import CtaBand from '$lib/CtaBand.svelte';
  import { CLONES, cloneDescription, cloneTitle } from '$lib/clones';
  import { GITHUB } from '$lib/links';
  import { APP_ID, absolute, breadcrumbs, faqPage, type Qa } from '$lib/site';

  let { data } = $props();
  const clone = $derived(data.clone);
  const path = $derived(`/clones/${clone.slug}`);
  const title = $derived(cloneTitle(clone));
  const others = $derived(CLONES.filter((other) => other.slug !== clone.slug));

  const faq = $derived<Qa[]>([
    {
      q: `Is there a free alternative to ${clone.plugin}?`,
      a: clone.video
        ? `Yes. In Powermove, the free video editor you customize with AI, the one-sentence prompt "${clone.prompt}" produced a working ${clone.plugin}-style effect with its own controls. ${clone.plugin} itself costs ${clone.price} (${clone.priceNote}).`
        : `Powermove, the free video editor you customize with AI, can build effects like ${clone.plugin} from a one-sentence prompt to a coding agent. ${clone.plugin} itself costs ${clone.price} (${clone.priceNote}).`,
    },
    {
      q: `Does this run the real ${clone.plugin} plugin?`,
      a: `No. Powermove does not run ${clone.vendor} plugins. A coding agent writes a new effect that recreates the look, inside Powermove.`,
    },
  ]);

  const graph = $derived([
    breadcrumbs([{ name: 'Powermove', path: '/' }, { name: 'Effect clones', path: '/clones' }, { name: clone.plugin, path }]),
    {
      '@type': 'WebPage',
      '@id': absolute(path),
      url: absolute(path),
      name: title,
      description: cloneDescription(clone),
      about: { '@id': APP_ID },
      ...(clone.video ? { primaryImageOfPage: absolute(clone.video.poster) } : {}),
    },
    ...(clone.video
      ? [{
          '@type': 'VideoObject',
          name: title,
          description: clone.video.description,
          thumbnailUrl: absolute(clone.video.poster),
          contentUrl: absolute(clone.video.src),
          uploadDate: clone.video.uploaded,
          duration: clone.video.duration,
        }]
      : []),
    faqPage(faq, path),
  ]);
</script>

<Seo {title} description={cloneDescription(clone)} {path} {graph} />

<Header />

<main class="page clone">
  <nav class="crumbs" aria-label="Breadcrumb">
    <a href="/clones">Effect clones</a><span aria-hidden="true">/</span><span aria-current="page">{clone.plugin}</span>
  </nav>
  <h1 class="page-title">{title}</h1>
  <p class="page-lead">
    <span class="strong">{clone.plugin} by {clone.vendor} costs {clone.price}.</span>
    {clone.what}
    {#if clone.video}In Powermove, the video editor you customize with AI, one sentence to a coding agent rebuilt the look.{:else}Here is how Powermove, the video editor you customize with AI, rebuilds it from one sentence to a coding agent.{/if}
  </p>

  <div class="clone-top">
    <Short {clone} />

    <div class="clone-side">
      <section class="side-block" aria-labelledby="prompt-title">
        <h2 id="prompt-title" class="label">{clone.video ? 'The whole prompt' : 'The prompt'}</h2>
        <div class="composer">
          <p class="composer-text">{clone.prompt}</p>
          <div class="composer-meta">
            {#if clone.agent}
              <span>{clone.agent.provider}</span><span>{clone.agent.model}</span><span>{clone.agent.effort}</span>
            {:else}
              <span>Claude or Codex</span>
            {/if}
            <span class="composer-send" aria-hidden="true"><svg viewBox="0 0 16 16"><path d="M12.5 4v4.5a1.5 1.5 0 0 1-1.5 1.5H4M6.5 7.5 4 10l2.5 2.5" /></svg></span>
          </div>
        </div>
        <p class="side-note">
          {#if clone.video}One sentence, typed into the agent panel beside the timeline. No plugin, no preset, no code written by hand.{:else}The short of this prompt is in production. You can run the same sentence in Powermove today.{/if}
        </p>
      </section>

      <dl class="facts card">
        <div><dt>Original</dt><dd>{clone.plugin} by {clone.vendor}</dd></div>
        <div><dt>Its price</dt><dd>{clone.price}, {clone.priceNote}</dd></div>
        <div><dt>In Powermove</dt><dd>Free and open source</dd></div>
        {#if clone.agent}<div><dt>Built by</dt><dd>{clone.agent.provider} {clone.agent.model}, from one sentence</dd></div>{/if}
      </dl>

      <section class="side-block" aria-labelledby="extension-title">
        <h2 id="extension-title" class="label">The free extension</h2>
        {#if clone.extensionUrl}
          <p class="side-note">Install the effect in Powermove and use it on your own footage.</p>
          <a class="pill dark" href={clone.extensionUrl}>Get the free extension</a>
        {:else}
          <p class="side-note"><span class="strong">Coming soon.</span> We’re packaging this effect so you can install it in one click. Until then, paste the prompt above into Powermove and your agent will build its own.</p>
          <a class="pill line" href={GITHUB} target="_blank" rel="noopener">Follow on GitHub</a>
        {/if}
      </section>
    </div>
  </div>

  <section class="page-section" aria-labelledby="how-title">
    <h2 id="how-title" class="rows-title"><span class="strong">How it works.</span> From a sentence to an effect you can keyframe.</h2>
    <ol class="steps">
      <li class="card"><h3>Ask in one sentence</h3><p>Type the prompt into Powermove’s agent panel, next to the timeline. It works with Claude Code, Codex, or an OpenAI-compatible endpoint.</p></li>
      <li class="card"><h3>The agent writes an effect</h3><p>It builds a Powermove extension: the code that draws the look, plus its controls in the Effects panel. Every edit goes through the same undoable tools you use.</p></li>
      <li class="card"><h3>Tune it like any effect</h3><p>Apply it to a layer and adjust the controls. Each one can be keyframed, so the look can change over time.</p></li>
      <li class="card"><h3>Render it</h3><p>Export MP4, WebM, or ProRes with sound, a still, or a PNG sequence.</p></li>
    </ol>
  </section>

  {#if clone.controls}
    <section class="page-section" aria-labelledby="controls-title">
      <h2 id="controls-title" class="rows-title"><span class="strong">The controls it built.</span> {clone.controls.length} of them, in the Effects panel like any built-in effect.</h2>
      <ul class="chips" aria-label="{clone.plugin} clone controls">
        {#each clone.controls as control (control)}<li>{control}</li>{/each}
      </ul>
    </section>
  {/if}

  <section class="page-section" aria-labelledby="clone-faq-title">
    <h2 id="clone-faq-title" class="rows-title"><span class="strong">Questions.</span></h2>
    <dl class="faq-list">
      {#each faq as item (item.q)}
        <div class="faq-item"><dt>{item.q}</dt><dd>{item.a}</dd></div>
      {/each}
    </dl>
  </section>

  <CtaBand title="Make your own clone." text="Powermove is the video editor you customize with AI. Free and open source, for Macs with Apple silicon." />

  <nav class="page-section more" aria-label="More">
    <p class="label">Keep reading</p>
    <ul>
      {#each others as other (other.slug)}
        <li><a href="/clones/{other.slug}">{cloneTitle(other)}</a></li>
      {/each}
      <li><a href="/vs/after-effects">Powermove vs After Effects</a></li>
      <li><a href="/#faq">What is a malleable video editor?</a></li>
    </ul>
  </nav>
</main>

<Footer />

<style>
  .clone-top{display:grid;gap:32px;margin-top:clamp(48px,6vw,72px)}
  @media(min-width:900px){.clone-top{grid-template-columns:minmax(0,5fr) minmax(0,7fr);gap:48px;align-items:start}}
  .clone-side{display:grid;gap:32px;align-content:start}
  .side-block{display:grid;gap:12px;justify-items:start}
  .side-note{max-width:46ch;font-size:15px;line-height:1.5;color:var(--tx-2);text-wrap:pretty}
  .side-note .strong{color:var(--tx);font-weight:500}

  /* The agent composer as it looks in the app's dark theme. */
  .composer{display:grid;gap:18px;width:100%;max-width:440px;box-sizing:border-box;padding:16px 16px 12px 18px;border-radius:14px;background:#1C1917;box-shadow:inset 0 0 0 1px rgb(255 255 255 / .08),0 18px 36px -18px rgb(28 18 12 / .5)}
  .composer-text{font-size:17px;line-height:1.4;color:#F5F5F4}
  .composer-meta{display:flex;align-items:center;gap:14px;font-size:13px;color:#8A847E}
  .composer-send{display:grid;place-items:center;width:28px;height:28px;margin-left:auto;border-radius:8px;background:var(--accent)}
  .composer-send svg{width:14px;height:14px;fill:none;stroke:#fff;stroke-width:1.6;stroke-linecap:round;stroke-linejoin:round}

  .facts{margin:0;width:100%}
  .facts div{display:grid;grid-template-columns:minmax(0,4fr) minmax(0,7fr);gap:16px;padding:14px 20px;border-top:1px solid var(--line)}
  .facts div:first-child{border-top:0}
  .facts dt{font-size:var(--fs-lg);color:var(--tx-3)}
  .facts dd{margin:0;font-size:var(--fs-lg);color:var(--tx);text-wrap:pretty}

  .chips{display:flex;flex-wrap:wrap;gap:8px}
  .chips li{padding:8px 12px;border-radius:8px;background:#fff;box-shadow:0 0 0 1px rgb(28 18 12 / .08);font-size:var(--fs-lg);color:var(--tx)}

  .more ul{display:grid;gap:4px}
  .more a{font-size:17px;color:var(--tx);text-decoration:underline;text-decoration-color:var(--line-strong);text-underline-offset:3px}
  .more a:hover{text-decoration-color:var(--tx)}
</style>
