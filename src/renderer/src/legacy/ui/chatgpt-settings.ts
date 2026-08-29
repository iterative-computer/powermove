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

export function createChatGPTSettingsControl(
  api?: PowermoveBridge['chatgpt']
): ChatGPTSettingsControl {
  const accountApi = api ?? window.powermove?.chatgpt;
  const row = document.createElement('div');
  row.className = 'settings-row settings-chatgpt';
  const copy = document.createElement('div');
  copy.className = 'settings-copy';
  const title = document.createElement('b');
  title.textContent = 'ChatGPT';
  const description = document.createElement('span');
  description.textContent = 'Checking your Codex sign-in…';
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
    action.classList.toggle('is-connected', status.state === 'connected');
    if (status.state === 'connected') {
      const identity = [status.email, planLabel(status.planType)].filter(Boolean).join(' · ');
      description.textContent = identity || 'Your ChatGPT subscription is ready to use.';
      action.textContent = canDisconnect ? 'Disconnect' : 'Connected';
      action.disabled = !canDisconnect;
    } else if (status.state === 'connecting') {
      description.textContent = status.detail || 'Finish signing in in your browser.';
      action.textContent = 'Waiting…';
      action.disabled = true;
    } else if (status.state === 'checking') {
      description.textContent = 'Checking your Codex sign-in…';
      action.textContent = 'Checking…';
      action.disabled = true;
    } else {
      description.textContent = status.detail || (status.state === 'unavailable'
        ? 'Codex could not be reached on this Mac.'
        : 'Use your ChatGPT subscription with Powermove.');
      action.textContent = status.state === 'unavailable' ? 'Retry' : 'Connect';
      action.disabled = false;
    }
  };

  if (!accountApi) {
    render({
      state: 'unavailable',
      email: null,
      planType: null,
      detail: 'Restart Powermove once to finish installing ChatGPT connection.'
    });
    action.textContent = 'Restart needed';
    action.disabled = true;
    return {
      element: row,
      focus: () => undefined,
      destroy: () => { alive = false; }
    };
  }

  const stop = accountApi.onChanged(render);
  action.addEventListener('click', async () => {
    if (busy || currentState === 'connecting') return;
    busy = true;
    if (currentState === 'connected' && canDisconnect) {
      description.textContent = 'Disconnecting ChatGPT from Powermove…';
      action.classList.remove('is-connected');
      action.textContent = 'Disconnecting…';
      action.disabled = true;
      try {
        render(await accountApi.disconnect());
      } catch (error) {
        render({
          state: 'unavailable',
          email: null,
          planType: null,
          detail: error instanceof Error ? error.message : 'ChatGPT could not be disconnected.'
        });
      } finally {
        busy = false;
      }
      return;
    }
    render({ state: 'connecting', email: null, planType: null, detail: 'Opening ChatGPT sign-in…' });
    try {
      render(await accountApi.connect());
    } catch (error) {
      render({
        state: 'unavailable',
        email: null,
        planType: null,
        detail: error instanceof Error ? error.message : 'ChatGPT could not be connected.'
      });
    } finally {
      busy = false;
    }
  });
  void accountApi.status().then(render, (error) => render({
    state: 'unavailable',
    email: null,
    planType: null,
    detail: error instanceof Error ? error.message : 'Codex could not be reached on this Mac.'
  }));

  return {
    element: row,
    focus: () => action.focus(),
    destroy: () => { alive = false; stop(); }
  };
}

export const formatChatGPTPlan = planLabel;
