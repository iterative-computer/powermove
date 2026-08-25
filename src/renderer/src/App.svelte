<script lang="ts">
  let count = $state(0);
  let pingResult = $state('Checking bridge…');

  const versions = window.powermove.versions;

  async function loadPing(): Promise<void> {
    try {
      pingResult = await window.powermove.ping();
    } catch {
      pingResult = 'unavailable';
    }
  }

  void loadPing();
</script>

<svelte:head>
  <meta name="theme-color" content="#0b0b0c" />
</svelte:head>

<main>
  <section aria-labelledby="scaffold-title">
    <p class="eyebrow">electron-vite · Svelte 5</p>
    <h1 id="scaffold-title">Powermove scaffold</h1>
    <p class="lede">The isolated renderer and typed preload bridge are connected.</p>

    <div class="actions">
      <button type="button" onclick={() => (count += 1)}>Count: {count}</button>
      <p class="status" role="status">IPC: <strong>{pingResult}</strong></p>
    </div>

    <dl aria-label="Runtime versions">
      <div>
        <dt>Electron</dt>
        <dd>{versions.electron}</dd>
      </div>
      <div>
        <dt>Chrome</dt>
        <dd>{versions.chrome}</dd>
      </div>
      <div>
        <dt>Node</dt>
        <dd>{versions.node}</dd>
      </div>
    </dl>
  </section>
</main>

<style>
  :global(*) {
    box-sizing: border-box;
  }

  :global(html) {
    color-scheme: dark;
    background: #0b0b0c;
    font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Inter", sans-serif;
  }

  :global(body) {
    min-width: 320px;
    min-height: 100vh;
    margin: 0;
    background:
      radial-gradient(circle at 50% -10%, rgba(255, 107, 26, 0.11), transparent 35rem),
      #0b0b0c;
    color: #ededee;
  }

  main {
    display: grid;
    min-height: 100vh;
    place-items: center;
    padding: 5rem 2rem 2rem;
  }

  section {
    width: min(36rem, 100%);
    padding: 2rem;
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 1rem;
    background: rgba(20, 20, 22, 0.92);
    box-shadow: 0 1.5rem 4rem rgba(0, 0, 0, 0.36);
  }

  .eyebrow {
    margin: 0 0 0.75rem;
    color: #ff8b4d;
    font-size: 0.75rem;
    font-weight: 650;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  h1 {
    margin: 0;
    font-size: clamp(1.75rem, 4vw, 2.5rem);
    letter-spacing: -0.035em;
    line-height: 1.05;
  }

  .lede {
    max-width: 30rem;
    margin: 0.85rem 0 1.75rem;
    color: #9b9ba1;
    line-height: 1.55;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 1rem;
  }

  button {
    min-height: 2.5rem;
    padding: 0.65rem 1rem;
    border: 0;
    border-radius: 0.6rem;
    background: #ff6b1a;
    color: #1a0900;
    font: inherit;
    font-weight: 700;
    cursor: pointer;
  }

  button:hover {
    background: #ff8247;
  }

  button:focus-visible {
    outline: 2px solid #ffffff;
    outline-offset: 3px;
  }

  .status {
    margin: 0;
    color: #9b9ba1;
  }

  .status strong {
    color: #3fcf8e;
  }

  dl {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.75rem;
    margin: 2rem 0 0;
  }

  dl div {
    min-width: 0;
    padding: 0.8rem;
    border-radius: 0.5rem;
    background: #101012;
  }

  dt {
    color: #6a6a70;
    font-size: 0.72rem;
    font-weight: 650;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }

  dd {
    margin: 0.3rem 0 0;
    overflow-wrap: anywhere;
    font-family: "SF Mono", ui-monospace, monospace;
    font-size: 0.85rem;
    font-variant-numeric: tabular-nums;
  }

  @media (prefers-reduced-motion: no-preference) {
    button {
      transition-property: background-color, transform;
      transition-duration: 120ms;
      transition-timing-function: ease-out;
    }

    button:active {
      transform: scale(0.96);
    }
  }

  @media (max-width: 32rem) {
    main {
      padding-inline: 1rem;
    }

    section {
      padding: 1.5rem;
    }

    .actions {
      align-items: flex-start;
      flex-direction: column;
    }

    dl {
      grid-template-columns: 1fr;
    }
  }
</style>
