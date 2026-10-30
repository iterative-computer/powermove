import type { PowermoveBridge } from '../../../../shared/ipc';
import { COMPATIBLE_PRESETS, DEFAULT_COMPATIBLE_PROVIDER, presetFor, type CompatibleModelInfo, type CompatiblePreset, type CompatibleProviderConfig } from '../../../../shared/compatible-provider';
import { bridge as hostBridge } from '../../kernel/bridge';

const GROUPS: Array<[kind: CompatiblePreset['kind'], label: string]> = [
  ['local', 'On this Mac'],
  ['cloud', 'Cloud, with an API key'],
  ['other', 'Custom']
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

function button(text: string): HTMLButtonElement {
  const node = el('button', 'btn', text);
  node.type = 'button';
  return node;
}

/** IPC errors arrive wrapped as "Error invoking remote method '…': Error: …". */
const message = (error: unknown, fallback: string): string =>
  error instanceof Error && error.message ? error.message.replace(/^Error invoking remote method '[^']+': (?:Error: )?/, '') : fallback;

const plural = (count: number, word: string): string => `${count} ${word}${count === 1 ? '' : 's'}`;

/**
 * The API or local model card. Pick a provider, add a key only where one is
 * needed, then pick a model from the provider's own list (or type one). Local
 * servers that are already running are found without asking.
 */
export function createCompatibleSettingsControl(api: PowermoveBridge['compatible'] = hostBridge()?.compatible) {
  const element = el('div', 'settings-provider-fields');

  const preset = el('select', 'settings-select');
  preset.setAttribute('aria-label', 'Provider');
  for (const [kind, label] of GROUPS) {
    const group = el('optgroup');
    group.label = label;
    for (const item of COMPATIBLE_PRESETS.filter(entry => entry.kind === kind)) {
      const option = el('option', '', item.name);
      option.value = item.id;
      group.append(option);
    }
    preset.append(group);
  }

  const url = input('API address', 'text', 'https://…/v1');
  const key = input('API key', 'password');
  const getKey = button('Get a key');
  const keyControl = el('div', 'settings-field-pair settings-provider-pair');
  keyControl.append(key, getKey);

  const listId = `pm-compatible-models-${Math.random().toString(36).slice(2)}`;
  const model = input('Model', 'text', 'Model name');
  model.setAttribute('list', listId);
  const options = el('datalist');
  options.id = listId;
  const refresh = button('Refresh');
  const download = button('Download');
  download.hidden = true;
  const modelControl = el('div', 'settings-field-pair settings-provider-pair');
  modelControl.append(model, options, download, refresh);

  const vision = el('input');
  vision.type = 'checkbox';
  vision.setAttribute('aria-label', 'This model supports images');

  const action = button('Test and connect');
  const status = el('span');
  status.setAttribute('role', 'status');

  const provider = row('Provider', '', preset);
  const address = row('Address', 'Change it if the server uses another port, or runs on another machine through a gateway.', url);
  const keyRow = row('API key', '', keyControl);
  const modelRow = row('Model', '', modelControl);
  const images = row('Images', 'The model can read pictures you attach to a message.', vision);
  const test = el('div', 'settings-row');
  const testCopy = el('div', 'settings-copy');
  testCopy.append(el('b', '', 'Status'), status);
  test.append(testCopy, action);
  element.append(provider.element, address.element, keyRow.element, modelRow.element, images.element, test);

  let alive = true, edited = false, busy = false, hasKey = false, loadSeq = 0;
  let connected: CompatibleProviderConfig | null = null;
  let known = new Map<string, CompatibleModelInfo>();
  const current = (): CompatiblePreset => COMPATIBLE_PRESETS.find(item => item.id === preset.value) ?? COMPATIBLE_PRESETS.at(-1)!;
  const NOT_CONNECTED = 'Not connected.';

  /** Show only what this kind of provider needs. */
  const layout = (): void => {
    const chosen = current();
    address.element.hidden = chosen.kind === 'cloud';
    keyRow.element.hidden = chosen.kind === 'local';
    getKey.hidden = !chosen.keyUrl;
    provider.hint.textContent = chosen.kind === 'local'
      ? `Runs on this Mac: private, free, and works offline.`
      : chosen.kind === 'cloud'
        ? `Pay as you go with your ${chosen.name} API key, billed by ${chosen.name}, not your ChatGPT or Claude plan.`
        : 'Any OpenAI-compatible API, gateway, or server you run yourself.';
    key.placeholder = hasKey ? 'Saved' : chosen.keyPrefix ? `${chosen.keyPrefix}…` : 'Paste your API key';
    keyRow.hint.textContent = hasKey
      ? 'Saved securely on this Mac. Leave blank to keep it.'
      : chosen.kind === 'cloud' ? 'Saved securely on this Mac.' : 'Leave blank if the server doesn’t need one.';
  };

  const showStatus = (): void => {
    status.textContent = connected?.model
      ? connected.warning ? `Connected: ${connected.model}. ${connected.warning}` : `Connected: ${connected.model}`
      : NOT_CONNECTED;
  };

  const setModels = (models: CompatibleModelInfo[]): void => {
    known = new Map(models.map(item => [item.id, item]));
    options.replaceChildren(...models.map(item => { const option = el('option'); option.value = item.id; return option; }));
  };

  /** Ask the provider which models it has, so nobody has to type an exact name. */
  const loadModels = async (): Promise<void> => {
    const chosen = current();
    const seq = ++loadSeq;
    download.hidden = true;
    if (!api?.models || !url.value.trim()) { setModels([]); modelRow.hint.textContent = 'The exact model name your provider expects.'; return; }
    modelRow.hint.textContent = 'Looking for models…';
    try {
      const list = await api.models({ baseUrl: url.value, ...(key.value.trim() ? { apiKey: key.value } : {}) });
      if (!alive || seq !== loadSeq) return;
      hasKey = list.hasKey;
      layout();
      setModels(list.models);
      const hidden = list.hidden ? ` ${plural(list.hidden, 'other')} can’t use tools, so ${list.hidden === 1 ? 'it’s' : 'they’re'} not listed.` : '';
      modelRow.hint.textContent = list.models.length
        ? `${plural(list.models.length, 'model')} available. Pick one or type its name.${hidden}`
        : chosen.kind === 'local' && list.hidden
          ? `None of your models can use tools, which the agent needs. Download one that can, such as Qwen 3 or Llama 3.1.`
          : chosen.kind === 'local'
            ? `${chosen.name} has no models yet. Download one in ${chosen.name}, then Refresh.`
            : 'Type the exact model name your provider expects.';
      if (!model.value && list.models.length === 1) pick(list.models[0]!.id);
    } catch (error) {
      if (!alive || seq !== loadSeq) return;
      setModels([]);
      const text = message(error, 'Could not list models.');
      modelRow.hint.textContent = chosen.kind === 'cloud' && !hasKey && !key.value.trim() && /key/i.test(text)
        ? 'Add your API key to see the models you can use.'
        : text;
      download.hidden = !(chosen.kind === 'local' && chosen.downloadUrl && /isn’t running|isn’t answering/.test(text));
    }
  };

  const pick = (id: string): void => {
    model.value = id;
    const info = known.get(id);
    if (info?.vision !== undefined) vision.checked = info.vision;
  };

  const choosePreset = (id: string): void => {
    preset.value = id;
    const chosen = current();
    url.value = chosen.baseUrl;
    key.value = '';
    hasKey = false;
    model.value = connected && connected.baseUrl === url.value ? connected.model : '';
    setModels([]);
    layout();
    showStatus();
    void loadModels();
  };

  element.addEventListener('input', () => { edited = true; });
  preset.onchange = () => { edited = true; choosePreset(preset.value); if (current().kind === 'other') url.focus(); };
  url.addEventListener('change', () => { void loadModels(); });
  key.addEventListener('change', () => { void loadModels(); });
  model.addEventListener('input', () => { if (known.has(model.value)) pick(model.value); });
  refresh.onclick = () => { void loadModels(); };
  getKey.onclick = () => { const link = current().keyUrl; if (link) void hostBridge()?.openExternal?.(link); };
  download.onclick = () => { const link = current().downloadUrl; if (link) void hostBridge()?.openExternal?.(link); };

  const render = (config: CompatibleProviderConfig) => {
    if (!alive) return;
    connected = config.model ? config : null;
    hasKey = config.hasKey;
    preset.value = presetFor(config.baseUrl).id;
    url.value = config.baseUrl;
    model.value = config.model;
    vision.checked = config.vision;
    key.value = '';
    layout();
    showStatus();
    void loadModels();
  };

  /* Nothing connected yet: if a local server is already running, start there. */
  const detect = async (): Promise<void> => {
    if (!api?.detect) return;
    const servers = await api.detect().catch(() => []);
    if (!alive || edited || connected || !servers.length) return;
    const server = servers[0]!;
    choosePreset(server.preset);
    status.textContent = `Found ${presetFor(server.baseUrl).name} running on this Mac. Pick a model, then test and connect.`;
  };

  preset.value = presetFor(DEFAULT_COMPATIBLE_PROVIDER.baseUrl).id;
  url.value = DEFAULT_COMPATIBLE_PROVIDER.baseUrl;
  layout();
  status.textContent = NOT_CONNECTED;

  if (api) {
    void api.status().then(config => { if (!edited) render(config); if (!config.model) void detect(); }, () => {
      if (alive) status.textContent = 'Could not read the connection. Enter its details to reconnect.';
    });
    action.onclick = async () => {
      if (busy) return;
      if (!model.value.trim()) { status.textContent = 'Pick a model first.'; model.focus(); return; }
      busy = true; action.disabled = true; action.textContent = 'Testing…'; status.textContent = 'Testing the connection…';
      try {
        const config = await api.configure({ baseUrl: url.value, model: model.value, apiKey: key.value, vision: vision.checked });
        render(config);
        window.dispatchEvent(new CustomEvent('pm-provider-connected', { detail: config }));
      } catch (error) {
        if (alive) status.textContent = message(error, 'Could not connect. Check the address and model.');
      } finally {
        busy = false;
        if (alive) { action.disabled = false; action.textContent = 'Test and connect'; }
      }
    };
  } else {
    action.disabled = true;
    refresh.disabled = true;
    status.textContent = 'Restart Powermove to enable API and local connections.';
  }

  return { element, destroy: () => { alive = false; }, focus: () => model.focus() };
}
