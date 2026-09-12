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
  { tool: 'anchor', icon: 'anchor', title: 'Anchor Point Tool (Y)', command: 'toolAnchor' },
  { tool: 'shape', icon: 'shape', title: 'Shape Tool (Q) · press Q again to cycle', command: 'toolShape' },
  { tool: 'pen', icon: 'pen', title: 'Pen Tool (G) · drag tangents · double-click to close · Command-click to delete vertex', command: 'toolPen' },
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
  api.services?.register('tool', {
    get tool() { return PM.tool!; },
    set tool(value: string) { PM.tool = value; },
    get toolShape() { return PM.toolShape!; },
    set toolShape(value: string) { PM.toolShape = value; },
    setTool(tool: string, detail?: string): void { PM.setTool?.(tool, detail); }
  });

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
      const groups: { button: HTMLButtonElement; definitions: ToolbarButton[]; current: ToolbarButton }[] = [];
      const choose = (definition: ToolbarButton): void => {
        // Clicking Shape preserves the current variant; Q still cycles variants.
        if (definition.tool === 'shape') PM.setTool?.('shape', PM.toolShape);
        else api.commands.run(definition.command);
        syncTools();
      };
      const syncTools = (): void => {
        for (const group of groups) {
          const active = group.definitions.find((definition) => definition.tool === PM.tool);
          if (active) group.current = active;
          const { button, current } = group;
          button.dataset.tool = current.tool;
          button.title = current.title;
          button.setAttribute('aria-label', current.title);
          button.setAttribute('aria-pressed', String(Boolean(active)));
          button.innerHTML = api.ui.icon(current.icon);
          button.classList.toggle('on', Boolean(active));
        }
      };
      const appendGroup = (name: string, definitions: ToolbarButton[]): void => {
        const first = definitions[0];
        if (!first) return;
        const wrapper = document.createElement('span');
        wrapper.className = 'tl-group';
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'iconbtn tl';
        const group = { button, definitions, current: first };
        groups.push(group);
        button.addEventListener('click', () => choose(group.current));
        button.addEventListener('dblclick', () => {
          if (group.current.tool === 'anchor') api.commands.run('centerAnchor');
        });
        wrapper.appendChild(button);
        if (definitions.length > 1) {
          const open = (): void => api.ui.menu(wrapper, definitions.map((definition) => ({
            icon: definition.icon,
            label: definition.title.split(' · ')[0] ?? definition.title,
            on: PM.tool === definition.tool,
            run: () => { group.current = definition; choose(definition); }
          })));
          const more = document.createElement('button');
          more.type = 'button';
          more.className = 'iconbtn tl-more';
          more.title = name;
          more.setAttribute('aria-label', name);
          more.setAttribute('aria-haspopup', 'menu');
          more.innerHTML = '<svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true"><path d="m2 3 2 2 2-2" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/></svg>';
          more.addEventListener('click', open);
          button.addEventListener('keydown', (event) => {
            if (event.key === 'ArrowDown') { event.preventDefault(); open(); }
          });
          wrapper.appendChild(more);
        }
        body.appendChild(wrapper);
      };

      appendGroup('Selection and transform tools', INTERACTIVE.filter(({ tool }) => ['select', 'rotate', 'anchor'].includes(tool!)));
      appendGroup('Navigation tools', INTERACTIVE.filter(({ tool }) => ['hand', 'zoom'].includes(tool!)));
      appendGroup('Drawing tools', INTERACTIVE.filter(({ tool }) => ['shape', 'pen'].includes(tool!)));
      appendGroup('Text tool', INTERACTIVE.filter(({ tool }) => tool === 'text'));
      const separator = document.createElement('span');
      separator.className = 'tl-sep';
      body.appendChild(separator);
      const add = document.createElement('button');
      add.type = 'button';
      add.className = 'iconbtn tl';
      add.title = 'Add layer or media';
      add.setAttribute('aria-label', add.title);
      add.setAttribute('aria-haspopup', 'menu');
      add.innerHTML = api.ui.icon('plus');
      add.addEventListener('click', () => api.ui.menu(add, [IMPORT, ...CREATE].map((definition) => ({
        icon: definition.icon,
        label: definition.title,
        run: () => api.commands.run(definition.command)
      }))));
      body.appendChild(add);

      const off = PM.bus?.on?.('tool', syncTools);
      syncTools();
      return () => off?.();
    }
  });
}
