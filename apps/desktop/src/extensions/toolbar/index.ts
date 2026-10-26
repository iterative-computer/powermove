import type { PowermoveAPI, ToolService } from 'powermove';

interface ToolbarButton {
  tool?: string;
  icon: string;
  title: string;
  command: string;
}

/* One button per tool, in After Effects' order and grouping. Layers are made
   from Layer ▸ New and the New ▸ context menus, not from the toolbar. */
const TOOL_GROUPS: ToolbarButton[][] = [
  [
    { tool: 'select', icon: 'cursor', title: 'Selection Tool (V)', command: 'toolSelect' },
    { tool: 'hand', icon: 'hand', title: 'Hand Tool (H)', command: 'toolHand' },
    { tool: 'zoom', icon: 'zoom', title: 'Zoom Tool (Z) · Option-click to zoom out', command: 'toolZoom' },
    { tool: 'rotate', icon: 'rotate', title: 'Rotation Tool (W)', command: 'toolRotate' }
  ],
  [
    { tool: 'anchor', icon: 'anchor', title: 'Pan Behind (Anchor Point) Tool (Y) · double-click to center', command: 'toolAnchor' }
  ],
  [
    { tool: 'shape', icon: 'shape', title: 'Shape Tool (Q) · press Q again to cycle', command: 'toolShape' },
    { tool: 'pen', icon: 'pen', title: 'Pen Tool (G) · drag tangents · double-click to close · Command-click to delete vertex', command: 'toolPen' }
  ],
  [
    { tool: 'text', icon: 'type', title: 'Horizontal Type Tool (Command+T)', command: 'toolText' }
  ]
];

const SHAPES: Array<{ id: string; label: string }> = [
  { id: 'rect', label: 'Rectangle Tool' },
  { id: 'rounded', label: 'Rounded Rectangle Tool' },
  { id: 'ellipse', label: 'Ellipse Tool' },
  { id: 'polygon', label: 'Polygon Tool' },
  { id: 'star', label: 'Star Tool' }
];

export default function activate(api: PowermoveAPI): void {
  let tool = 'select';
  let toolShape = 'rect';
  const toolListeners = new Set<() => void>();
  const toolService: ToolService = {
    get tool() { return tool; },
    set tool(value: string) { tool = value; },
    get toolShape() { return toolShape; },
    set toolShape(value: string) { toolShape = value; },
    setTool(nextTool: string, detail?: string): void {
      tool = nextTool;
      if (nextTool === 'shape' && detail) toolShape = detail;
      for (const listener of toolListeners) listener();
      // The viewer draws tool-specific affordances, such as the anchor marker.
      api.transport.invalidate();
    }
  };
  api.services.register<ToolService>('tool', toolService);

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
      const buttons: { button: HTMLButtonElement; definition: ToolbarButton }[] = [];
      const choose = (definition: ToolbarButton): void => {
        // Clicking Shape keeps the current variant; Q still cycles variants.
        if (definition.tool === 'shape') toolService.setTool('shape', toolService.toolShape);
        else api.commands.run(definition.command);
        syncTools();
      };
      const syncTools = (): void => {
        for (const { button, definition } of buttons) {
          const active = definition.tool === toolService.tool;
          button.setAttribute('aria-pressed', String(active));
          button.classList.toggle('on', active);
        }
      };
      const shapeMenu = (anchor: HTMLElement): void => {
        api.ui.menu(anchor, SHAPES.map((shape) => ({
          label: shape.label,
          on: toolService.tool === 'shape' && toolService.toolShape === shape.id,
          run: () => { toolService.setTool('shape', shape.id); syncTools(); }
        })));
      };
      const appendTool = (parent: HTMLElement, definition: ToolbarButton): void => {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'iconbtn tl';
        button.dataset.tool = definition.tool;
        button.title = definition.title;
        button.setAttribute('aria-label', definition.title.split(' · ')[0] ?? definition.title);
        button.innerHTML = api.ui.icon(definition.icon);
        button.addEventListener('click', () => choose(definition));
        if (definition.tool === 'anchor') button.addEventListener('dblclick', () => api.commands.run('centerAnchor'));
        buttons.push({ button, definition });
        if (definition.tool !== 'shape') { parent.appendChild(button); return; }
        // Shape is the one tool with variants: a small arrow beside it lists them.
        const wrapper = document.createElement('span');
        wrapper.className = 'tl-group';
        const more = document.createElement('button');
        more.type = 'button';
        more.className = 'iconbtn tl-more';
        more.title = 'Shape tools';
        more.setAttribute('aria-label', 'Shape tools');
        more.setAttribute('aria-haspopup', 'menu');
        more.innerHTML = api.ui.icon('chevD');
        more.addEventListener('click', () => shapeMenu(wrapper));
        button.addEventListener('keydown', (event) => {
          if (event.key === 'ArrowDown') { event.preventDefault(); shapeMenu(wrapper); }
        });
        wrapper.append(button, more);
        parent.appendChild(wrapper);
      };
      TOOL_GROUPS.forEach((group, index) => {
        if (index) {
          const separator = document.createElement('span');
          separator.className = 'tl-sep';
          body.appendChild(separator);
        }
        for (const definition of group) appendTool(body, definition);
      });

      toolListeners.add(syncTools);
      syncTools();
      return () => void toolListeners.delete(syncTools);
    }
  });
}
