<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import type { ApiErrorBody, MeDto } from '@powermove/registry/wire';
  import Icon from '../panels/Icon.svelte';
  import iconLight from './icon-light.png';
  import iconDark from './icon-dark.png';
  import { CLOUD_HANDLE, CLOUD_OTP, type CloudBridge, type CloudSocialProvider } from '../../../shared/cloud-ipc';
  import { applyAccount, subscribeAccount, userFromMe, PROVIDER_LABEL, type SignInMode } from './account';

  let { PM, mode: startMode, bridge, me: startMe, onclose }: {
    PM: Record<string, any>;
    mode: SignInMode;
    bridge: CloudBridge;
    /** The account when the sheet opens: signed in without a handle starts on the handle step. */
    me: MeDto | null;
    onclose: () => void;
  } = $props();

  /* One sheet, four steps. Start offers the providers and email; a provider
     hands off to the browser and main finishes the sign-in when the browser
     sends Powermove back; email asks for a code. Whenever the account has no
     handle yet, the sheet ends by claiming one, the publisher namespace on
     the Store. */
  type Step = 'start' | 'browser' | 'code' | 'handle';
  type HandleProblem = 'invalid' | 'taken' | 'reserved' | null;

  const PROVIDERS: CloudSocialProvider[] = ['google'];
  const CODE_LENGTH = 6;
  const RESEND_AFTER = 30;
  const TERMS = 'https://trypowermove.com/terms';
  const PRIVACY = 'https://trypowermove.com/privacy';

  const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

  const initialMe = untrack(() => startMe);
  let mode = $state<SignInMode>(untrack(() => startMode));
  let step = $state<Step>(initialMe && !initialMe.publisher ? 'handle' : 'start');
  let direction = $state(0);
  let provider = $state<CloudSocialProvider>('google');
  let browserError = $state<string | null>(null);
  let email = $state('');
  let emailError = $state<string | null>(null);
  let sending = $state(false);
  let code = $state('');
  let codeFocused = $state(false);
  let verifying = $state(false);
  let codeError = $state<string | null>(null);
  let resendIn = $state(0);
  let handle = $state(initialMe ? suggestHandle(initialMe) : '');
  let handleProblem = $state<HandleProblem>(null);
  let handleError = $state<string | null>(null);
  let claiming = $state(false);
  let done = false;
  let root: HTMLElement;
  let codeInput = $state<HTMLInputElement | null>(null);

  const emailValid = $derived(/^[^\s@]+@[^\s@]+\.[^\s@]{2,}$/.test(email.trim()));
  const providerLabel = $derived(PROVIDER_LABEL[provider]);
  const handleValid = $derived(CLOUD_HANDLE.test(handle));
  const canClaim = $derived(handleValid && !claiming);
  /* Pushing a step slides in from the right, going back from the left, as
     the Store's detail pages do. The first step just appears. */
  const slide = $derived(reduced() || direction === 0 ? { duration: 0 } : { x: direction * 16, duration: 200, easing: cubicOut });
  const handleCopy = $derived.by(() => {
    if (handleError) return handleError;
    if (handleProblem === 'taken') return `@${handle} is taken. Try another.`;
    if (handleProblem === 'reserved') return `@${handle} is reserved. Try another.`;
    if (!handle) return 'Lowercase letters, numbers and hyphens.';
    if (handleProblem === 'invalid' || !handleValid) return 'Use 2 to 39 characters, starting with a letter or number.';
    return 'Lowercase letters, numbers and hyphens.';
  });
  /* Red only once the registry or a submit said no; typing shows the rule. */
  const handleState = $derived(handleError || handleProblem ? 'error' : 'hint');

  function suggestHandle(value: MeDto): string {
    const base = value.user.name?.trim() || value.user.email.split('@')[0] || '';
    return base.toLowerCase().replace(/[^a-z0-9-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 39);
  }

  /** Plain words for what the registry said. */
  function describe(error: ApiErrorBody, fallback: string): string {
    switch (error.error) {
      case 'rate_limited': return 'Too many attempts. Wait a minute, then try again.';
      case 'client_too_old': return 'This version of Powermove is too old to sign in. Update Powermove, then try again.';
      case 'internal': return error.detail ?? fallback;
      default: return fallback;
    }
  }

  /** A thrown error's own words (the served host explains itself), else the fallback. */
  function thrown(error: unknown, fallback: string): string {
    const message = error instanceof Error ? error.message : '';
    return message && !message.startsWith('Error invoking remote method') ? message : fallback;
  }

  function focusStep(): void {
    void tick().then(() => (root?.querySelector<HTMLElement>('[data-autofocus]') ?? root)?.focus({ preventScroll: true }));
  }

  function go(next: Step, towards: number): void {
    direction = towards;
    step = next;
    focusStep();
  }

  function back(): void {
    verifying = false;
    sending = false;
    browserError = null;
    go('start', -1);
  }

  /** The account arrived (browser, email or claim). Finish, or ask for a handle. */
  function arrived(next: MeDto | null): void {
    if (done || !next) return;
    if (next.publisher) {
      done = true;
      PM.toast?.(`Signed in as ${userFromMe(next).name}`, 2200, { kind: 'status', icon: 'userCircle' });
      onclose();
      return;
    }
    if (step !== 'handle') {
      if (!handle) handle = suggestHandle(next);
      go('handle', 1);
    }
  }

  /* ── providers: the browser does the asking ── */
  async function viaProvider(next: CloudSocialProvider): Promise<void> {
    provider = next;
    browserError = null;
    go('browser', 1);
    try {
      const result = await bridge.signInSocial({ provider: next });
      if (!result.ok) browserError = describe(result.error, `Unable to open ${PROVIDER_LABEL[next]}. Try again.`);
    } catch (error) {
      browserError = thrown(error, `Unable to open ${PROVIDER_LABEL[next]}. Try again.`);
    }
  }

  /* ── email: a six-digit code ── */
  async function send(): Promise<boolean> {
    sending = true;
    emailError = null;
    try {
      const result = await bridge.emailSend({ email });
      if (result.ok) return true;
      emailError = describe(result.error, 'Unable to send a code to this address. Check it and try again.');
    } catch (error) {
      emailError = thrown(error, 'Unable to send a code. Check your connection and try again.');
    } finally {
      sending = false;
    }
    return false;
  }

  async function sendCode(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!emailValid || sending) return;
    email = email.trim();
    if (!(await send())) return;
    code = '';
    codeError = null;
    resendIn = RESEND_AFTER;
    go('code', 1);
  }

  async function resend(): Promise<void> {
    code = '';
    codeError = null;
    if (await send()) resendIn = RESEND_AFTER;
    else codeError = emailError;
    codeInput?.focus();
  }

  function typeCode(event: Event): void {
    const input = event.currentTarget as HTMLInputElement;
    code = input.value.replace(/\D/g, '').slice(0, CODE_LENGTH);
    input.value = code;
    codeError = null;
    if (CLOUD_OTP.test(code)) void verify();
  }

  async function verify(): Promise<void> {
    verifying = true;
    codeInput?.blur();
    try {
      const result = await bridge.emailVerify({ email, otp: code });
      if (result.ok) {
        applyAccount(result.value);
        arrived(result.value);
        return;
      }
      codeError = result.error.error === 'unauthorized'
        ? 'That code didn’t match. Check it, or send a new one.'
        : describe(result.error, 'Unable to check the code. Try again.');
    } catch {
      codeError = 'Unable to check the code. Check your connection and try again.';
    } finally {
      verifying = false;
    }
    if (codeError) void tick().then(() => { codeInput?.focus(); codeInput?.select(); });
  }

  /* ── handle: the publisher namespace ── */
  function typeHandle(event: Event): void {
    const input = event.currentTarget as HTMLInputElement;
    handle = input.value.toLowerCase().replace(/[^a-z0-9-]/g, '').slice(0, 39);
    input.value = handle;
    handleProblem = null;
    handleError = null;
  }

  async function claim(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!canClaim) {
      if (!handleValid) handleProblem = 'invalid';
      return;
    }
    claiming = true;
    handleError = null;
    try {
      const result = await bridge.claimHandle({ handle });
      if (result.ok) {
        applyAccount(result.value);
        arrived(result.value);
        return;
      }
      switch (result.error.error) {
        case 'handle_taken': handleProblem = 'taken'; break;
        case 'handle_reserved': handleProblem = 'reserved'; break;
        case 'handle_invalid': handleProblem = 'invalid'; break;
        case 'handle_already_set': {
          // Claimed from another Mac meanwhile: take the account as it is.
          const account = await bridge.account();
          applyAccount(account.me);
          arrived(account.me);
          break;
        }
        default: handleError = describe(result.error, 'Unable to save your handle. Try again.');
      }
    } catch {
      handleError = 'Unable to save your handle. Check your connection and try again.';
    } finally {
      claiming = false;
    }
  }

  function external(url: string): void {
    const host = (window as any).powermove;
    if (host?.openExternal) void host.openExternal(url);
    else window.open(url, '_blank', 'noopener');
  }

  $effect(() => {
    if (step !== 'code' || resendIn <= 0) return;
    const tickDown = window.setTimeout(() => { resendIn -= 1; }, 1000);
    return () => window.clearTimeout(tickDown);
  });

  onMount(() => {
    // The browser sign-in finishes in main; the account arrives as a broadcast.
    let first = true;
    const offAccount = subscribeAccount((_user, next) => {
      if (first) { first = false; return; }
      arrived(next);
    });
    const offFailed = bridge.onSignInFailed((error) => {
      if (step !== 'browser') return;
      browserError = describe(error, 'Unable to finish signing in. Try again.');
    });
    if (step === 'handle') focusStep();
    return () => { offAccount(); offFailed(); };
  });
</script>

<!-- The sheet itself takes focus on open, so nothing is lit before you act.
     Later steps move focus to their one field. -->
<!-- svelte-ignore a11y_autofocus -->
<div class="acct-sheet" bind:this={root} tabindex="-1" autofocus>
  {#key step}
    <div class="acct-step" in:fly={slide}>
      {#if step === 'start'}
        <img class="acct-mark is-light" src={iconLight} alt="" width="48" height="48" draggable="false" />
        <img class="acct-mark is-dark" src={iconDark} alt="" width="48" height="48" draggable="false" />
        {#key mode}
          <header class="acct-head" in:fade={{ duration: reduced() ? 0 : 120 }}>
            <h2>{mode === 'sign-in' ? 'Sign in to Powermove' : 'Create your Powermove account'}</h2>
            <p>{mode === 'sign-in'
              ? 'Publish and manage your extensions on the Store.'
              : 'Everything you publish to the Store lives under your handle.'}</p>
          </header>
        {/key}
        <div class="acct-providers">
          {#each PROVIDERS as p (p)}
            <button class="btn acct-provider" type="button" data-provider={p} onclick={() => void viaProvider(p)}>
              <Icon {PM} name={p} />
              <span>Continue with {PROVIDER_LABEL[p]}</span>
            </button>
          {/each}
        </div>
        <form class="acct-email" novalidate onsubmit={sendCode}>
          <input
            class="acct-field"
            type="email"
            placeholder="Email address"
            aria-label="Email address"
            aria-invalid={!!emailError}
            aria-describedby={emailError ? 'acct-email-error' : undefined}
            autocomplete="email"
            spellcheck="false"
            bind:value={email}
            oninput={() => (emailError = null)}
          />
          {#if emailError}<p class="acct-status is-wrong" id="acct-email-error" role="alert">{emailError}</p>{/if}
          <button class="btn pri acct-submit" type="submit" disabled={!emailValid || sending}>
            {sending ? 'Sending code…' : 'Continue with email'}
          </button>
        </form>
        <p class="acct-legal">
          By continuing, you agree to the
          <button class="acct-link" type="button" onclick={() => external(TERMS)}>Terms</button>
          and
          <button class="acct-link" type="button" onclick={() => external(PRIVACY)}>Privacy Policy</button>.
        </p>
        <footer class="acct-foot">
          <p>
            {mode === 'sign-in' ? 'New to Powermove?' : 'Already have an account?'}
            <button class="acct-link" type="button" onclick={() => (mode = mode === 'sign-in' ? 'sign-up' : 'sign-in')}>
              {mode === 'sign-in' ? 'Create an account' : 'Sign in'}
            </button>
          </p>
          <button class="btn ghost" type="button" onclick={onclose}>Cancel</button>
        </footer>

      {:else if step === 'browser'}
        <button class="acct-back" type="button" onclick={back}><Icon {PM} name="chev" /><span>Back</span></button>
        <header class="acct-head">
          <h2>Finish signing in in your browser</h2>
          <p>Powermove opened {providerLabel} in your browser. When you’re done there, you’ll come straight back here.</p>
        </header>
        <div class="acct-wait" role="status">
          {#if browserError}
            <span class="acct-status is-wrong">{browserError}</span>
            <button class="btn ghost" type="button" data-autofocus onclick={() => void viaProvider(provider)}>Try again</button>
          {:else}
            <svg class="acct-spinner" viewBox="0 0 16 16" aria-hidden="true">
              {#each { length: 12 } as _, i}
                <line x1="8" y1="1.75" x2="8" y2="4.5" transform={`rotate(${i * 30} 8 8)`} opacity={0.16 + 0.84 * ((i + 1) / 12)} />
              {/each}
            </svg>
            <span>Waiting for {providerLabel}…</span>
            <button class="btn ghost" type="button" data-autofocus onclick={onclose}>Cancel</button>
          {/if}
        </div>

      {:else if step === 'code'}
        <button class="acct-back" type="button" onclick={back}><Icon {PM} name="chev" /><span>Back</span></button>
        <header class="acct-head">
          <h2>Check your email</h2>
          <p>Enter the 6-digit code sent to <b>{email}</b>.</p>
        </header>
        <label class="acct-code" class:wrong={!!codeError} class:verifying>
          <input
            bind:this={codeInput}
            type="text"
            inputmode="numeric"
            autocomplete="one-time-code"
            maxlength={CODE_LENGTH}
            aria-label="Verification code"
            aria-invalid={!!codeError}
            data-autofocus
            disabled={verifying}
            value={code}
            oninput={typeCode}
            onfocus={() => (codeFocused = true)}
            onblur={() => (codeFocused = false)}
          />
          {#each { length: CODE_LENGTH } as _, i}
            <span
              class="acct-cell"
              class:filled={i < code.length}
              class:active={codeFocused && i === Math.min(code.length, CODE_LENGTH - 1)}
              aria-hidden="true"
            >{code[i] ?? ''}</span>
          {/each}
        </label>
        <div class="acct-code-foot">
          <span class="acct-status" class:is-wrong={!!codeError} role="status">
            {#if verifying}Checking the code…{:else if codeError}{codeError}{/if}
          </span>
          <button class="btn ghost" type="button" disabled={resendIn > 0 || verifying || sending} onclick={() => void resend()}>
            {resendIn > 0 ? `Resend in ${resendIn}s` : 'Resend code'}
          </button>
        </div>

      {:else}
        <header class="acct-head">
          <h2>Choose your handle</h2>
          <p>It’s your name on the Store. Extensions you publish live under it, and it can’t be changed later.</p>
        </header>
        <form class="sg-column acct-form" novalidate onsubmit={claim}>
          <div class="sg-group">
            <label class="settings-row">
              <span class="settings-copy">
                <b>Handle</b>
                <span class="acct-handle-status" data-state={handleState} role="status">{handleCopy}</span>
              </span>
              <span class="acct-handle-field">
                <span aria-hidden="true">@</span>
                <input
                  type="text"
                  aria-label="Handle"
                  aria-invalid={handleState === 'error'}
                  autocomplete="username"
                  autocapitalize="off"
                  spellcheck="false"
                  maxlength="39"
                  data-autofocus
                  value={handle}
                  oninput={typeHandle}
                />
              </span>
            </label>
          </div>
          <p class="acct-coord">Your extensions publish as <code>{handle || 'handle'}/my-extension</code></p>
          <button class="btn pri acct-submit" type="submit" disabled={!canClaim}>
            {claiming ? 'Saving handle…' : 'Choose handle'}
          </button>
          <button class="btn ghost acct-later" type="button" onclick={onclose}>Not now</button>
        </form>
      {/if}
    </div>
  {/key}
</div>
