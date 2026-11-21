<script lang="ts">
  import { onMount } from 'svelte';
  import { bridge } from '../kernel/bridge';
  import { dismissNativeReleaseBanner, NATIVE_RELEASE_BANNER_KEY, shouldShowNativeReleaseBanner } from './native-release-banner';

  let { launch }: { launch?: string } = $props();
  let visible = $state(false);
  let dismissedInWindow = false;

  function refresh() {
    if (dismissedInWindow) return;
    try {
      visible = shouldShowNativeReleaseBanner(launch, localStorage.getItem(NATIVE_RELEASE_BANNER_KEY), Date.now());
    } catch {
      visible = false;
    }
  }

  function dismiss() {
    visible = false;
    try {
      const saved = localStorage.getItem(NATIVE_RELEASE_BANNER_KEY);
      const next = dismissNativeReleaseBanner(saved, Date.now());
      if (next && next !== saved) localStorage.setItem(NATIVE_RELEASE_BANNER_KEY, next);
    } catch {
      dismissedInWindow = true;
    }
  }

  function download() {
    void bridge()?.openExternal?.('https://trypowermove.com/#download');
  }

  onMount(() => {
    if (!launch) return;
    refresh();
    const timer = window.setInterval(refresh, 60_000);
    return () => window.clearInterval(timer);
  });
</script>

<svelte:window onfocus={refresh} onstorage={(event) => {
  if (event.key === null || event.key === NATIVE_RELEASE_BANNER_KEY) refresh();
}} />

{#if visible}
  <aside class="toast native-release" aria-label="The new Powermove is here">
    <div class="copy">
      <strong>The new Powermove is here</strong>
      <span>Faster, native Mac app. Your projects come with you.</span>
    </div>
    <button type="button" class="toast-action" onclick={download}>Download</button>
    <button type="button" aria-label="Dismiss Powermove announcement" onclick={dismiss}>×</button>
  </aside>
{/if}

<style>
  .native-release {
    max-width: none;
    margin: 0 var(--pad) var(--gap);
    -webkit-app-region: no-drag;
  }

  .copy {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    line-height: 1.35;
  }

  .copy strong { font-weight: var(--fw-semibold); }
  .copy span { color: var(--tx-2); }
  .native-release > button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
