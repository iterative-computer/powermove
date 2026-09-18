<script lang="ts">
  import Header from '$lib/Header.svelte';
  import { GITHUB } from '$lib/links';

  const updated = 'September 15, 2026';
</script>

<svelte:head>
  <title>Terms of Service · Powermove</title>
  <meta name="description" content="The terms for using Powermove. Short version: it's free, open source, provided as is, and you're responsible for what you make with it and for your AI provider accounts." />
  <link rel="canonical" href="https://trypowermove.com/terms" />
</svelte:head>

<Header />

<main class="wrap legal">
  <h1>Terms of Service</h1>
  <p class="meta">Last updated {updated}</p>

  <h2>Who we are</h2>
  <p>Powermove is made by Iterative Computer. By downloading or using the app, or using trypowermove.com, you agree to these terms. If you don't agree, don't use it.</p>

  <h2>License</h2>
  <p>
    The Powermove source code is available <a href={GITHUB} target="_blank" rel="noopener">on GitHub</a> under the <a href="https://www.gnu.org/licenses/gpl-3.0.html" target="_blank" rel="noopener">GNU General Public License v3.0</a>. That license controls what you may do with the code, including running, studying, modifying, and redistributing it. These terms cover your use of the built app and this website, and add nothing that restricts rights the GPL grants you. Where these terms and the GPL conflict about the code, the GPL wins.
  </p>

  <h2>Your content</h2>
  <p>
    Projects, media, and exports you create with Powermove belong to you. We don't receive them and claim no rights to them. You're responsible for having the rights to any media you import and for anything you publish.
  </p>

  <h2>AI providers</h2>
  <p>
    The agent features run on third-party models using credentials you supply. You're responsible for those accounts, their costs, and complying with the provider's terms, including
    <a href="https://www.anthropic.com/legal/consumer-terms" target="_blank" rel="noopener">Anthropic's</a> and
    <a href="https://openai.com/policies/terms-of-use" target="_blank" rel="noopener">OpenAI's</a>.
    Model output can be wrong. Review it before you rely on it or ship it.
  </p>

  <h2>Acceptable use</h2>
  <p>Don't use Powermove to violate any law, infringe anyone's rights, create content that harasses or deceives people, or interfere with the website, GitHub releases, or other users.</p>

  <h2>No warranty</h2>
  <p>
    Powermove is provided "as is" and "as available," without warranties of any kind, express or implied, including fitness for a particular purpose and non-infringement. The agent can create, modify, and delete files in your projects. Keep backups. We don't promise the software will be error-free or that updates will always be available.
  </p>

  <h2>Limitation of liability</h2>
  <p>
    To the fullest extent permitted by law, Iterative Computer and its contributors are not liable for any indirect, incidental, special, consequential, or punitive damages, or for lost data, lost profits, or the cost of substitute services, arising from your use of Powermove. Our total liability for any claim is limited to the amount you paid us for the software, which is currently zero.
  </p>

  <h2>Changes and termination</h2>
  <p>
    We may update these terms or stop distributing Powermove at any time. Changes take effect when posted here with a new date. You can stop using the app whenever you like, and the source license continues to apply to the code regardless.
  </p>

  <h2>Governing law</h2>
  <p>These terms are governed by the laws of the jurisdiction where Iterative Computer is established, without regard to conflict of law rules. If any part of these terms is unenforceable, the rest still applies.</p>

  <h2>Contact</h2>
  <p>Iterative Computer · <a href="mailto:hello@iterative.computer">hello@iterative.computer</a> · <a href="/privacy">Privacy Policy</a></p>
</main>

<style>
  .legal { max-width: 68ch; padding-block: 120px 96px; }
  .legal h1 { font-size: clamp(30px, 4vw, 44px); }
  .meta { margin-top: 12px; color: var(--tx-3); font-size: var(--fs-md); }
  .legal h2 { margin-top: 40px; font-size: var(--fs-xl); letter-spacing: -0.01em; }
  .legal p { margin-top: 12px; font-size: var(--fs-lg); line-height: 1.6; color: var(--tx-2); text-wrap: pretty; }
  .legal a { color: var(--tx); text-decoration: underline; text-decoration-color: var(--line-strong); text-underline-offset: 3px; }
  .legal a:hover { text-decoration-color: var(--tx); }
</style>
