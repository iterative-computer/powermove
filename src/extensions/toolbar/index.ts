import type { PowermoveAPI } from 'powermove';

interface LegacyPM {
  tool?: string;
  setTool?: (tool: string) => void;
  bus?: {
    emit?: (event: string) => void;
    on?: (event: string, listener: () => void) => void | (() => void);
  };
}

interface ToolbarButton {
  tool: string;
  icon: string;
  title: string;
  command: string;
}

const INTERACTIVE: ToolbarButton[] = [
  { tool: 'select', icon: 'cursor', title: 'Selection (V)', command: 'toolSelect' },
  { tool: 'hand', icon: 'hand', title: 'Hand — pan view (H)', command: 'toolHand' },
  { tool: 'zoom', icon: 'zoom', title: 'Zoom (Z)', command: 'toolZoom' }
];

const CREATE: ToolbarButton[] = [
  { tool: 'text', icon: 'type', title: 'New text layer (⌘T)', command: 'newText' },
  { tool: 'shape', icon: 'shape', title: 'New shape layer (⌘⇧Y)', command: 'newShape' },
  { tool: 'solid', icon: 'solid', title: 'New solid (⌘Y)', command: 'newSolid' },
  { tool: 'shader', icon: 'wand', title: 'New shader layer (⌘⇧G)', command: 'newShader' },
  { tool: 'null', icon: 'frame', title: 'New null object', command: 'newNull' }
];

const IMPORT: ToolbarButton = { tool: 'camera', icon: 'cam', title: 'Import media (⌘I)', command: 'import' };

export default function activate(api: PowermoveAPI): void {
  const PM = api.host.pm as LegacyPM;
  PM.tool ||= 'select';
  PM.setTool = (tool: string): void => {
    PM.tool = tool;
    PM.bus?.emit?.('tool');
  };

  api.panels.register({
    id: 'toolbar',
    title: 'Tools',
    headless: true,
    flush: true,
    size: 40,
    noscroll: true,
    build(body) {
      body.id = 'toolbar';
      const interactiveButtons: HTMLButtonElement[] = [];

      const appendButton = (definition: ToolbarButton, interactive = false): void => {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'iconbtn tl';
        button.title = definition.title;
        button.setAttribute('aria-label', definition.title);
        button.dataset.tool = definition.tool;
        button.innerHTML = api.ui.icon(definition.icon);
        button.addEventListener('click', () => {
          api.commands.run(definition.command);
          syncTools();
        });
        if (interactive) interactiveButtons.push(button);
        body.appendChild(button);
      };

      const appendSeparator = (): void => {
        const separator = document.createElement('span');
        separator.className = 'tl-sep';
        body.appendChild(separator);
      };

      const syncTools = (): void => {
        for (const button of interactiveButtons) button.classList.toggle('on', button.dataset.tool === PM.tool);
      };

      for (const definition of INTERACTIVE) appendButton(definition, true);
      appendSeparator();
      for (const definition of CREATE) appendButton(definition);
      appendSeparator();
      appendButton(IMPORT);

      const off = PM.bus?.on?.('tool', syncTools);
      syncTools();
      return () => off?.();
    }
  });
}
