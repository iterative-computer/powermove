import type { ChatGPTAccountStatus, PowermoveBridge } from '../../../../shared/ipc';

function planLabel(plan: string | null): string | null {
  if (!plan) return null;
  const words = plan.replace(/[_-]+/gu, ' ').trim();
  return words ? `${words.replace(/\b\w/gu, (letter) => letter.toUpperCase())} plan` : null;
}

export interface ChatGPTSettingsControl {
  element: HTMLElement;
  focus(): void;
  destroy(): void;
}

type AccountApi = PowermoveBridge['chatgpt'] | PowermoveBridge['claude'];

function createAccountSettingsControl(
  name: 'ChatGPT' | 'Claude',
  runtime: 'ChatGPT' | 'Claude Code',
  accountApi: AccountApi | undefined
): ChatGPTSettingsControl {
  const row = document.createElement('div');
  row.className = 'settings-row settings-provider';
  const copy = document.createElement('div');
  copy.className = 'settings-copy';
  const title = document.createElement('b');
  title.className = 'settings-provider-title';
  const titleText = document.createElement('span');
  titleText.textContent = name;
  const dot = document.createElement('i');
  dot.className = 'settings-dot';
  dot.setAttribute('aria-hidden', 'true');
  title.append(titleText, dot);
  const description = document.createElement('span');
  description.textContent = `Checking your ${runtime} sign-in…`;
  const action = document.createElement('button');
  action.type = 'button';
  action.className = 'btn';
  action.disabled = true;
  action.textContent = 'Checking…';
  copy.append(title, description);
  row.append(copy, action);

  let alive = true;
  let busy = false;
  let currentState: ChatGPTAccountStatus['state'] = 'checking';
  const canDisconnect = typeof accountApi?.disconnect === 'function';

  const render = (status: ChatGPTAccountStatus): void => {
    if (!alive) return;
    currentState = status.state;
    dot.dataset.state = status.state;
    if (status.state === 'connected') {
      const identity = [status.email, planLabel(status.planType)].filter(Boolean).join(' · ');
      description.textContent = identity || `Your ${name} subscription is ready to use.`;
      action.textContent = canDisconnect ? 'Disconnect' : 'Connected';
      action.disabled = !canDisconnect;
    } else if (status.state === 'connecting') {
      description.textContent = status.detail || 'Finish signing in in your browser.';
      action.textContent = 'Waiting…';
      action.disabled = true;
    } else if (status.state === 'checking') {
      description.textContent = `Checking your ${runtime} sign-in…`;
      action.textContent = 'Checking…';
      action.disabled = true;
    } else {
      description.textContent = status.detail || (status.state === 'unavailable'
        ? `${runtime} could not be reached on this Mac.`
        : `Use your ${name} subscription with Powermove.`);
      action.textContent = status.state === 'unavailable' ? 'Retry' : 'Connect';
      action.disabled = false;
    }
  };

  if (!accountApi) {
    render({
      state: 'unavailable', email: null, planType: null,
      detail: `Restart Powermove once to finish installing ${name} connection.`
    });
    action.textContent = 'Restart needed';
    action.disabled = true;
    return { element: row, focus: () => undefined, destroy: () => { alive = false; } };
  }

  const stop = accountApi.onChanged(render);
  action.addEventListener('click', async () => {
    if (busy || currentState === 'connecting') return;
    busy = true;
    if (currentState === 'connected' && canDisconnect) {
      description.textContent = `Disconnecting ${name} from Powermove…`;
      action.textContent = 'Disconnecting…';
      action.disabled = true;
      try { render(await accountApi.disconnect()); }
      catch (error) {
        render({
          state: 'unavailable', email: null, planType: null,
          detail: error instanceof Error ? error.message : `${name} could not be disconnected.`
        });
      } finally { busy = false; }
      return;
    }
    render({ state: 'connecting', email: null, planType: null, detail: `Opening ${name} sign-in…` });
    try { render(await accountApi.connect()); }
    catch (error) {
      render({
        state: 'unavailable', email: null, planType: null,
        detail: error instanceof Error ? error.message : `${name} could not be connected.`
      });
    } finally { busy = false; }
  });
  void accountApi.status().then(render, (error) => render({
    state: 'unavailable', email: null, planType: null,
    detail: error instanceof Error ? error.message : `${runtime} could not be reached on this Mac.`
  }));

  return {
    element: row,
    focus: () => action.focus(),
    destroy: () => { alive = false; stop(); }
  };
}

export function createChatGPTSettingsControl(
  api?: PowermoveBridge['chatgpt']
): ChatGPTSettingsControl {
  return createAccountSettingsControl('ChatGPT', 'ChatGPT', api ?? window.powermove?.chatgpt);
}

export function createClaudeSettingsControl(
  api?: PowermoveBridge['claude']
): ChatGPTSettingsControl {
  return createAccountSettingsControl('Claude', 'Claude Code', api ?? window.powermove?.claude);
}

export const formatChatGPTPlan = planLabel;
