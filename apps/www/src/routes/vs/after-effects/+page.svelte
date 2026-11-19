<script lang="ts">
  import Header from '$lib/Header.svelte';
  import Footer from '$lib/Footer.svelte';
  import Seo from '$lib/Seo.svelte';
  import CtaBand from '$lib/CtaBand.svelte';
  import { APP_ID, MALLEABLE, absolute, breadcrumbs, faqPage } from '$lib/site';
  import { VS_AE_DESCRIPTION as description, VS_AE_FAQ as faq, VS_AE_ROWS as rows, VS_AE_TITLE as title } from '$lib/compare';

  const path = '/vs/after-effects';

  const graph = [
    breadcrumbs([{ name: 'Powermove', path: '/' }, { name: 'Powermove vs After Effects', path }]),
    { '@type': 'WebPage', '@id': absolute(path), url: absolute(path), name: title, description, about: { '@id': APP_ID } },
    faqPage(faq, path),
  ];
</script>

<Seo {title} {description} {path} {graph} />

<Header />

<main class="page vs">
  <p class="crumbs"><span>Compare</span><span aria-hidden="true">/</span><span aria-current="page">After Effects</span></p>
  <h1 class="page-title">Powermove vs After Effects</h1>
  <p class="page-lead">
    <span class="strong">After Effects gives you a huge, fixed toolbox. Powermove is the video editor you customize with AI.</span>
    When it’s missing something, you don’t wait for a plugin: you ask a coding agent and it adds what you need.
  </p>

  <section class="page-section" aria-labelledby="short-title">
    <h2 id="short-title" class="rows-title"><span class="strong">The short version.</span> Pick by how you want to grow the tool.</h2>
    <div class="picks">
      <div class="card pick">
        <h3>Choose Powermove if</h3>
        <ul>
          <li>You want a free, open-source motion editor on your Mac.</li>
          <li>You’d rather describe a missing feature than shop for it.</li>
          <li>You already work with Claude, Codex, or another coding agent.</li>
          <li>You want the interface, keymap, and effects to fit how you work.</li>
        </ul>
      </div>
      <div class="card pick">
        <h3>Stay with After Effects if</h3>
        <ul>
          <li>You rely on specific third-party plugins or templates.</li>
          <li>Your studio’s pipeline is built on Adobe tools and .aep projects.</li>
          <li>You need Windows.</li>
          <li>You need a mature tool with decades of tutorials behind it.</li>
        </ul>
      </div>
    </div>
  </section>

  <section class="page-section" aria-labelledby="malleable-title">
    <h2 id="malleable-title" class="rows-title"><span class="strong">What “malleable” changes.</span> The editor isn’t finished when you install it.</h2>
    <div class="prose">
      <p>{MALLEABLE}</p>
      <p>Powermove keeps a deliberately tiny kernel: the project store, typed edits, undo, and the compositor. Panels, effects, transitions, themes, and keymaps are all extensions, including the ones we wrote. Your agent builds new ones through the same undoable tools, so a feature you ask for behaves like one that shipped.</p>
      <p>In After Effects, the equivalent is a plugin someone else wrote. Some cost more than the editor: <a href="/clones/sapphire-glint">Sapphire S_Glint</a> ships in a $1,865 suite, and in Powermove its look took one sentence.</p>
    </div>
  </section>

  <section class="page-section" aria-labelledby="table-title">
    <h2 id="table-title" class="rows-title"><span class="strong">Side by side.</span></h2>
    <div class="table-wrap card">
      <table>
        <thead>
          <tr><th scope="col"><span class="sr-only">Topic</span></th><th scope="col">Powermove</th><th scope="col">After Effects</th></tr>
        </thead>
        <tbody>
          {#each rows as row (row.topic)}
            <tr><th scope="row">{row.topic}</th><td>{row.powermove}</td><td>{row.ae}</td></tr>
          {/each}
        </tbody>
      </table>
    </div>
  </section>

  <section class="page-section" aria-labelledby="vs-faq-title">
    <h2 id="vs-faq-title" class="rows-title"><span class="strong">Questions.</span></h2>
    <dl class="faq-list">
      {#each faq as item (item.q)}
        <div class="faq-item"><dt>{item.q}</dt><dd>{item.a}</dd></div>
      {/each}
    </dl>
  </section>

  <CtaBand title="Try the editor that grows with you." />
</main>

<Footer />

<style>
  .picks{display:grid;gap:12px}
  @media(min-width:720px){.picks{grid-template-columns:repeat(2,minmax(0,1fr))}}
  .pick{padding:24px 24px 28px}
  .pick h3{font-size:17px;font-weight:500;color:var(--tx)}
  .pick ul{display:grid;gap:10px;margin-top:14px}
  .pick li{position:relative;padding-left:18px;font-size:15px;line-height:1.5;color:var(--tx-2);text-wrap:pretty}
  .pick li::before{content:'';position:absolute;left:2px;top:.62em;width:6px;height:6px;border-radius:50%;background:var(--tx-4)}
  .pick:first-child li::before{background:var(--accent)}
  .prose{display:grid;gap:16px}

  .table-wrap{overflow-x:auto}
  table{width:100%;min-width:600px;border-collapse:collapse;font-size:15px;line-height:1.5}
  th,td{padding:14px 20px;text-align:left;vertical-align:top;border-top:1px solid var(--line);text-wrap:pretty}
  thead th{border-top:0;font-size:var(--fs-md);font-weight:500;color:var(--tx-3)}
  thead th:nth-child(2){color:var(--accent-tx)}
  tbody th{width:22%;font-weight:500;color:var(--tx)}
  td{width:39%;color:var(--tx-2)}
</style>
