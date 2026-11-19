<script lang="ts">
  import Header from '$lib/Header.svelte';
  import Footer from '$lib/Footer.svelte';
  import Seo from '$lib/Seo.svelte';
  import CtaBand from '$lib/CtaBand.svelte';
  import { CLONES, cloneTitle } from '$lib/clones';
  import { APP_ID, absolute, breadcrumbs } from '$lib/site';

  const title = 'Free plugin alternatives, made in Powermove';
  const description =
    'Paid After Effects plugin looks like Sapphire S_Glint and Optical Flares, rebuilt in Powermove, the video editor you customize with AI, from one sentence each.';

  const graph = [
    breadcrumbs([{ name: 'Powermove', path: '/' }, { name: 'Effect clones', path: '/clones' }]),
    {
      '@type': 'CollectionPage',
      '@id': absolute('/clones'),
      url: absolute('/clones'),
      name: title,
      description,
      about: { '@id': APP_ID },
      mainEntity: {
        '@type': 'ItemList',
        itemListElement: CLONES.map((clone, i) => ({ '@type': 'ListItem', position: i + 1, url: absolute(`/clones/${clone.slug}`), name: cloneTitle(clone) })),
      },
    },
  ];
</script>

<Seo title="Free After Effects plugin alternatives, made in Powermove" {description} path="/clones" {graph} />

<Header />

<main class="page">
  <p class="crumbs"><span aria-current="page">Effect clones</span></p>
  <h1 class="page-title">{title}</h1>
  <p class="page-lead">
    <span class="strong">Some looks cost more than the software you edit in.</span>
    Powermove is the video editor you customize with AI, so we ask a coding agent for each one in a single sentence and publish what comes back: the short, the exact prompt, and the effect.
  </p>

  <ul class="clone-grid">
    {#each CLONES as clone (clone.slug)}
      <li>
        <a class="clone-card card" href="/clones/{clone.slug}">
          <span class="clone-media">
            {#if clone.video}
              <img src={clone.video.thumb} alt="" width="1080" height="608" loading="lazy" />
            {:else}
              <span class="clone-soon" aria-hidden="true"></span>
            {/if}
          </span>
          <span class="clone-copy">
            <span class="clone-name">{clone.plugin}</span>
            <span class="clone-meta">{clone.vendor} · {clone.price} · {clone.video ? 'Short published' : 'Short in production'}</span>
            <span class="clone-prompt">“{clone.prompt}”</span>
          </span>
        </a>
      </li>
    {/each}
  </ul>

  <CtaBand title="Make your own clone." text="Download Powermove, connect Claude or Codex, and describe the effect you would otherwise buy." />
</main>

<Footer />

<style>
  .clone-grid{display:grid;gap:16px;margin-top:clamp(48px,6vw,72px)}
  @media(min-width:720px){.clone-grid{grid-template-columns:repeat(2,minmax(0,1fr))}}
  .clone-card{display:grid;height:100%;transition:box-shadow var(--dur-3)}
  .clone-card:hover{box-shadow:0 0 0 1px rgb(28 18 12 / .12),0 28px 56px -28px rgb(28 18 12 / .3)}
  .clone-media{display:block;aspect-ratio:16/9;overflow:hidden;background:#0C0A09}
  .clone-media img{width:100%;height:100%;object-fit:cover}
  .clone-soon{display:block;width:100%;height:100%;background:radial-gradient(50% 60% at 50% 50%,color-mix(in oklab,var(--ember) 60%,transparent),transparent 70%)}
  .clone-copy{display:grid;gap:6px;padding:20px 22px 24px}
  .clone-name{font-size:19px;font-weight:500;letter-spacing:-.02em;color:var(--tx)}
  .clone-meta{font-size:var(--fs-lg);color:var(--tx-3)}
  .clone-prompt{margin-top:6px;font-size:15px;color:var(--tx-2)}
</style>
