import type { PowermoveBridge } from '../../../../shared/ipc';
import { DEFAULT_COMPATIBLE_PROVIDER } from '../../../../shared/compatible-provider';

export function createCompatibleSettingsControl(api: PowermoveBridge['compatible'] = window.powermove?.compatible) {
  const element = document.createElement('div'); element.className = 'settings-provider settings-sections';
  const heading = document.createElement('b'); heading.textContent = 'API or local model';
  const description = document.createElement('span');
  description.textContent = 'Connect an OpenAI-compatible API, Ollama, or LM Studio. API billing is separate from ChatGPT and Claude subscriptions.';
  const preset = document.createElement('select'); preset.setAttribute('aria-label', 'Connection type');
  for (const [value, name] of [['http://localhost:11434/v1', 'Ollama'], ['http://localhost:1234/v1', 'LM Studio'], ['https://api.openai.com/v1', 'OpenAI API'], ['', 'Other compatible provider']]) {
    const option = document.createElement('option'); option.value = value!; option.textContent = name!; preset.append(option);
  }
  const field = (name: string, type = 'text') => {
    const label = document.createElement('label'); label.className = 'field'; label.textContent = name;
    const input = document.createElement('input'); input.type = type; input.setAttribute('aria-label', name); input.autocomplete = 'off'; label.append(input);
    return { label, input };
  };
  const url = field('API base URL'), model = field('Model name'), key = field('API key', 'password');
  url.input.value = DEFAULT_COMPATIBLE_PROVIDER.baseUrl; model.input.placeholder = 'Exact model name from your provider';
  key.input.placeholder = 'Optional for local models';
  preset.onchange = () => { url.input.value = preset.value; key.input.value = ''; key.input.placeholder = preset.value.startsWith('http:') ? 'Optional for local models' : 'Enter your API key'; };
  const visionLabel = document.createElement('label');
  const vision = document.createElement('input'); vision.type = 'checkbox'; visionLabel.append(vision, ' This model supports images');
  const action = document.createElement('button'); action.className = 'btn'; action.textContent = 'Test and connect'; action.type = 'button';
  const status = document.createElement('span'); status.setAttribute('role', 'status');
  let alive = true, edited = false, busy = false;
  element.addEventListener('input', () => { edited = true; });
  const render = (config: typeof DEFAULT_COMPATIBLE_PROVIDER) => {
    if (!alive) return;
    url.input.value = config.baseUrl; model.input.value = config.model; vision.checked = config.vision;
    preset.value = [...preset.options].some(option => option.value === config.baseUrl) ? config.baseUrl : '';
    key.input.value = ''; key.input.placeholder = config.hasKey ? 'Key saved securely · leave blank to keep' : 'Optional for local models';
    status.textContent = config.model ? `Connected model: ${config.model}` : 'Start your local server, or enter your provider’s API details.';
  };
  if (api) {
    void api.status().then(config => { if (!edited) render(config); }, () => { if (alive) status.textContent = 'Could not read the connection. Enter its details to reconnect.'; });
    action.onclick = async () => {
      if (busy) return;
      busy = true; action.disabled = true; action.textContent = 'Testing connection…'; status.textContent = '';
      try {
        const config = await api.configure({ baseUrl: url.input.value, model: model.input.value, apiKey: key.input.value, vision: vision.checked });
        render(config);
        window.dispatchEvent(new CustomEvent('pm-provider-connected', { detail: config }));
      } catch (error) { if (alive) status.textContent = error instanceof Error ? error.message : 'Could not connect. Check the address and model.'; }
      finally { busy = false; if (alive) { action.disabled = false; action.textContent = 'Test and connect'; } }
    };
  } else { action.disabled = true; status.textContent = 'Restart Powermove to enable API and local connections.'; }
  element.append(heading, description, preset, url.label, model.label, key.label, visionLabel, action, status);
  return { element, destroy: () => { alive = false; }, focus: () => model.input.focus() };
}
