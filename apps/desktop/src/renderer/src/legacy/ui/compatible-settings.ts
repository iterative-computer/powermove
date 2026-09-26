import type { PowermoveBridge } from '../../../../shared/ipc';
import { DEFAULT_COMPATIBLE_PROVIDER } from '../../../../shared/compatible-provider';
import { bridge as hostBridge } from '../../kernel/bridge';

const PRESETS: Array<[url: string, name: string]> = [
  ['http://localhost:11434/v1', 'Ollama'],
  ['http://localhost:1234/v1', 'LM Studio'],
  ['https://api.openai.com/v1', 'OpenAI API'],
  ['', 'Other compatible provider']
];

const el = <K extends keyof HTMLElementTagNameMap>(tag: K, className = '', text = ''): HTMLElementTagNameMap[K] => {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text) node.textContent = text;
  return node;
};

/** One settings row: title and hint on the left, a single control on the right. */
function row(title: string, hint: string, control: HTMLElement): { element: HTMLElement; hint: HTMLElement } {
  const element = el('div', 'settings-row');
  const copy = el('div', 'settings-copy');
  const hintEl = el('span', '', hint);
  copy.append(el('b', '', title), hintEl);
  element.append(copy, control);
  return { element, hint: hintEl };
}

function input(label: string, type = 'text', placeholder = ''): HTMLInputElement {
  const field = el('input', 'settings-input is-wide');
  field.type = type;
  field.setAttribute('aria-label', label);
  field.autocomplete = 'off';
  field.spellcheck = false;
  field.placeholder = placeholder;
  return field;
}

/**
 * The API or local model card: connection preset, address, model, key, image
 * support, and one row that tests the connection and reports its state.
 */
export function createCompatibleSettingsControl(api: PowermoveBridge['compatible'] = hostBridge()?.compatible) {
  const element = el('div', 'settings-provider-fields');

  const preset = el('select', 'settings-select');
  preset.setAttribute('aria-label', 'Connection type');
  for (const [value, name] of PRESETS) {
    const option = el('option', '', name);
    option.value = value;
    preset.append(option);
  }

  const url = input('API base URL', 'text', 'https://…/v1');
  const model = input('Model name', 'text', 'e.g. llama3.1');
  const key = input('API key', 'password');
  const vision = el('input');
  vision.type = 'checkbox';
  vision.setAttribute('aria-label', 'This model supports images');

  const action = el('button', 'btn', 'Test and connect');
  action.type = 'button';
  const status = el('span');
  status.setAttribute('role', 'status');

  const connection = row('Connection', 'Ollama, LM Studio, or any OpenAI-compatible API. Billing is separate from ChatGPT and Claude.', preset);
  const address = row('Address', 'Base URL of the API, ending in /v1.', url);
  const modelRow = row('Model', 'The exact model name your provider expects.', model);
  const keyRow = row('API key', 'Optional for local models.', key);
  const images = row('Images', 'The model can read pictures you attach to a message.', vision);
  const test = el('div', 'settings-row');
  const testCopy = el('div', 'settings-copy');
  testCopy.append(el('b', '', 'Status'), status);
  test.append(testCopy, action);
  element.append(connection.element, address.element, modelRow.element, keyRow.element, images.element, test);

  url.value = DEFAULT_COMPATIBLE_PROVIDER.baseUrl;
  status.textContent = 'Not connected. Start your local server, or enter your provider’s details.';

  let alive = true, edited = false, busy = false;
  const local = (value: string) => value.startsWith('http:');
  const keyHint = (hasKey: boolean): string => hasKey
    ? 'Key saved securely. Leave blank to keep it.'
    : local(url.value) ? 'Optional for local models.' : 'Required by this provider.';

  element.addEventListener('input', () => { edited = true; keyRow.hint.textContent = keyHint(false); });
  preset.onchange = () => {
    url.value = preset.value;
    key.value = '';
    keyRow.hint.textContent = keyHint(false);
    edited = true;
  };

  const render = (config: typeof DEFAULT_COMPATIBLE_PROVIDER) => {
    if (!alive) return;
    url.value = config.baseUrl;
    model.value = config.model;
    vision.checked = config.vision;
    preset.value = [...preset.options].some(option => option.value === config.baseUrl) ? config.baseUrl : '';
    key.value = '';
    keyRow.hint.textContent = keyHint(config.hasKey);
    status.textContent = config.model
      ? `Connected model: ${config.model}`
      : 'Not connected. Start your local server, or enter your provider’s details.';
  };

  if (api) {
    void api.status().then(config => { if (!edited) render(config); }, () => {
      if (alive) status.textContent = 'Could not read the connection. Enter its details to reconnect.';
    });
    action.onclick = async () => {
      if (busy) return;
      busy = true; action.disabled = true; action.textContent = 'Testing…'; status.textContent = 'Testing the connection…';
      try {
        const config = await api.configure({ baseUrl: url.value, model: model.value, apiKey: key.value, vision: vision.checked });
        render(config);
        window.dispatchEvent(new CustomEvent('pm-provider-connected', { detail: config }));
      } catch (error) {
        if (alive) status.textContent = error instanceof Error ? error.message : 'Could not connect. Check the address and model.';
      } finally {
        busy = false;
        if (alive) { action.disabled = false; action.textContent = 'Test and connect'; }
      }
    };
  } else {
    action.disabled = true;
    status.textContent = 'Restart Powermove to enable API and local connections.';
  }

  return { element, destroy: () => { alive = false; }, focus: () => model.focus() };
}
