<script lang="ts">
  import Header from '$lib/Header.svelte';
  import { GITHUB } from '$lib/links';

  const updated = 'September 15, 2026';
</script>

<svelte:head>
  <title>Privacy Policy · Powermove</title>
  <meta name="description" content="How Powermove handles your data. Short version: your projects stay on your Mac, and your prompts go only to the AI provider you choose." />
  <link rel="canonical" href="https://trypowermove.com/privacy" />
</svelte:head>

<Header />

<main class="wrap legal">
  <h1>Privacy Policy</h1>
  <p class="meta">Last updated {updated}</p>

  <section class="tldr">
    <h2>TL;DR</h2>
    <ul>
      <li>Powermove is a desktop app. Your projects, media, and edits stay on your Mac.</li>
      <li>We don't have accounts, and we don't run analytics or telemetry.</li>
      <li>When you use the agent, your prompts and relevant project files go to the AI provider you configured: Anthropic, OpenAI, or a local or OpenAI-compatible endpoint you point it at. Their privacy policies apply to that data.</li>
      <li>The only thing the app contacts on its own is GitHub, to check for updates.</li>
    </ul>
  </section>

  <h2>What Powermove processes</h2>
  <p>
    Powermove opens and edits video projects on your computer. Project files, imported media, renders, and settings are stored locally. We never see them, and there is no Powermove server that receives them.
  </p>

  <h2>AI providers</h2>
  <p>
    The agent features are powered by Claude Code, Codex, or an OpenAI-compatible endpoint you configure. When you send a prompt, Powermove transmits that prompt and the files the agent needs to your chosen provider using your own credentials. We do not proxy, store, or log this traffic. How each provider retains and uses that data is governed by their terms:
    <a href="https://www.anthropic.com/privacy" target="_blank" rel="noopener">Anthropic</a>,
    <a href="https://openai.com/policies/privacy-policy" target="_blank" rel="noopener">OpenAI</a>.
    If you use a local model, nothing leaves your machine.
  </p>
  <p>
    API keys you enter are encrypted with the macOS keychain via Electron's safeStorage and stay on your device.
  </p>

  <h2>Updates</h2>
  <p>
    On launch, the app checks GitHub Releases for a newer version. This request contains your current app version and the standard information any HTTP request carries, such as your IP address. GitHub's <a href="https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement" target="_blank" rel="noopener">privacy statement</a> applies.
  </p>

  <h2>This website</h2>
  <p>
    trypowermove.com is a static site served by Cloudflare. We run no analytics and set no cookies. Cloudflare may log requests to operate the network. The download button links to GitHub Releases.
  </p>

  <h2>Children</h2>
  <p>Powermove is not directed at children under 13, and we do not knowingly collect information from them.</p>

  <h2>Changes</h2>
  <p>
    If we add something that touches your data, such as accounts or opt-in crash reports, we will update this page and the date above before it ships. The source is <a href={GITHUB} target="_blank" rel="noopener">on GitHub</a>, so you can see every change.
  </p>

  <h2>Contact</h2>
  <p>Iterative Computer · <a href="mailto:hello@iterative.computer">hello@iterative.computer</a></p>
</main>

<style>
  .legal { max-width: 68ch; padding-block: 120px 96px; }
  .legal h1 { font-size: clamp(30px, 4vw, 44px); }
  .meta { margin-top: 12px; color: var(--tx-3); font-size: var(--fs-md); }
  .legal h2 { margin-top: 40px; font-size: var(--fs-xl); letter-spacing: -0.01em; }
  .legal p { margin-top: 12px; font-size: var(--fs-lg); line-height: 1.6; color: var(--tx-2); text-wrap: pretty; }
  .legal a { color: var(--tx); text-decoration: underline; text-decoration-color: var(--line-strong); text-underline-offset: 3px; }
  .legal a:hover { text-decoration-color: var(--tx); }
  .tldr { margin-top: 32px; padding: 20px 24px; border: 1px solid var(--line); border-radius: 12px; background: var(--bg-panel); }
  .tldr h2 { margin-top: 0; font-size: var(--fs-md); text-transform: uppercase; letter-spacing: 0.06em; color: var(--tx-3); }
  .tldr ul { margin-top: 12px; display: grid; gap: 10px; padding-left: 18px; font-size: var(--fs-lg); line-height: 1.5; color: var(--tx); }
</style>
