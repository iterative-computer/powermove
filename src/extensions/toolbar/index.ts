import type { PowermoveAPI } from 'powermove';

interface LegacyPM {
  tool?: string;
  toolShape?: string;
  setTool?: (tool: string, detail?: string) => void;
  bus?: {
    emit?: (event: string) => void;
    on?: (event: string, listener: () => void) => void | (() => void);
  };
}

interface ToolbarButton {
  tool?: string;
  icon: string;
  title: string;
  command: string;
}

const INTERACTIVE: ToolbarButton[] = [
  { tool: 'select', icon: 'cursor', title: 'Selection Tool (V)', command: 'toolSelect' },
  { tool: 'hand', icon: 'hand', title: 'Hand Tool (H)', command: 'toolHand' },
  { tool: 'zoom', icon: 'zoom', title: 'Zoom Tool (Z) · Option-click to zoom out', command: 'toolZoom' },
  { tool: 'rotate', icon: 'rotate', title: 'Rotation Tool (W)', command: 'toolRotate' },
  { tool: 'anchor', icon: 'anchor', title: 'Pan Behind (Anchor Point) Tool (Y)', command: 'toolAnchor' },
  { tool: 'shape', icon: 'shape', title: 'Shape Tool (Q) · press Q again to cycle', command: 'toolShape' },
  { tool: 'text', icon: 'type', title: 'Horizontal Type Tool (Command+T)', command: 'toolText' }
];

/* Layer creation and file import are commands in After Effects, not tools.
   Keep them visibly separated so they never imply a persistent canvas mode. */
const CREATE: ToolbarButton[] = [
  { icon: 'solid', title: 'New solid (Command+Y)', command: 'newSolid' },
  { icon: 'wand', title: 'New shader layer (Command+Shift+G)', command: 'newShader' },
  { icon: 'frame', title: 'New null object (Command+Option+Shift+Y)', command: 'newNull' }
];

const IMPORT: ToolbarButton = { icon: 'image', title: 'Import media (Command+I)', command: 'import' };

export default function activate(api: PowermoveAPI): void {
  const PM = api.host.pm as LegacyPM;
  PM.tool ||= 'select';
  PM.toolShape ||= 'rect';
  PM.setTool = (tool: string, detail?: string): void => {
    PM.tool = tool;
    if (tool === 'shape' && detail) PM.toolShape = detail;
    PM.bus?.emit?.('tool');
  };

  api.panels.register({
    id: 'toolbar',
    icon: 'tools',
    title: 'Tools',
    headless: true,
    flush: true,
    size: 40,
    noscroll: true,
    library: false,
    build(body) {
      body.id = 'toolbar';
      const interactiveButtons: HTMLButtonElement[] = [];

      const appendButton = (definition: ToolbarButton, interactive = false): void => {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'iconbtn tl';
        button.title = definition.title;
        button.setAttribute('aria-label', definition.title);
        if (definition.tool) button.dataset.tool = definition.tool;
        button.innerHTML = api.ui.icon(definition.icon);
        button.addEventListener('click', () => {
          /* Q cycles shape variants; clicking the already-active Shape button
             simply keeps the visible variant selected, like AE's toolbar. */
          if (definition.tool === 'shape') PM.setTool?.('shape', PM.toolShape);
          else api.commands.run(definition.command);
          syncTools();
        });
        if (definition.tool === 'anchor') {
          button.addEventListener('dblclick', () => api.commands.run('centerAnchor'));
        }
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
