import type { PowermoveAPI, ToolService } from 'powermove';

type NavigationTool=Exclude<ReturnType<PowermoveAPI['scene3d']['getNavigationMode']>,'select'>;
type NavigationVariant=ReturnType<PowermoveAPI['scene3d']['getNavigationVariant']>;
const CAMERA_TOOLS: Array<{id:NavigationTool;icon:string;label:string;variants:Array<{id:NavigationVariant;label:string}>}> = [
  {id:'orbit',icon:'orbit',label:'Orbit',variants:[{id:'cursor',label:'Orbit Around Cursor'},{id:'scene',label:'Orbit Around Scene'},{id:'poi',label:'Orbit Around Camera POI'}]},
  {id:'pan',icon:'pan3d',label:'Pan',variants:[{id:'cursor',label:'Pan Under Cursor'},{id:'poi',label:'Pan Camera POI'}]},
  {id:'dolly',icon:'dolly',label:'Dolly',variants:[{id:'towards-cursor',label:'Dolly Towards Cursor'},{id:'cursor',label:'Dolly to Cursor'},{id:'poi',label:'Dolly to Camera POI'}]},
];

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
      api.scene3d.setNavigationMode('select');
      tool = nextTool;
      if (nextTool === 'shape' && detail) toolShape = detail;
      for (const listener of toolListeners) listener();
      // The viewer draws tool-specific affordances, such as the anchor marker.
      api.transport.invalidate();
    }
  };
  api.services.register<ToolService>('tool', toolService);

  const navigate=(mode:NavigationTool,cycle=false)=>{
    const definition=CAMERA_TOOLS.find(item=>item.id===mode)!;
    if(cycle){const choices=definition.variants,index=choices.findIndex(item=>item.id===api.scene3d.getNavigationVariant(mode));
      api.scene3d.setNavigationVariant(mode,choices[(index+1)%choices.length]!.id);}
    api.scene3d.setNavigationMode(mode);api.transport.invalidate();
  };
  CAMERA_TOOLS.forEach((definition,index)=>{
    api.commands.register({id:`tool3d${definition.label}`,label:`${definition.label} 3D View`,category:'Tools',kb:String(index+1),run:(cycle)=>navigate(definition.id,cycle===true)});
  });
  api.commands.register({id:'tool3dCycle',label:'Cycle 3D View Tools',category:'Tools',kb:'C',run:()=>{
    const index=CAMERA_TOOLS.findIndex(item=>item.id===api.scene3d.getNavigationMode());navigate(CAMERA_TOOLS[(index+1)%CAMERA_TOOLS.length]!.id);
  }});

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
      const cameraButtons: Array<{button:HTMLButtonElement;id:NavigationTool}> = [];
      const choose = (definition: ToolbarButton): void => {
        // Clicking Shape keeps the current variant; Q still cycles variants.
        if (definition.tool === 'shape') toolService.setTool('shape', toolService.toolShape);
        else api.commands.run(definition.command);
        syncTools();
      };
      const syncTools = (): void => {
        for (const { button, definition } of buttons) {
          const active = api.scene3d.getNavigationMode()==='select' && definition.tool === toolService.tool;
          button.setAttribute('aria-pressed', String(active));
          button.classList.toggle('on', active);
        }
      };
      const syncCamera=():void=>{
        syncTools();
        for(const {button,id} of cameraButtons){
          const definition=CAMERA_TOOLS.find(item=>item.id===id)!,variant=definition.variants.find(item=>item.id===api.scene3d.getNavigationVariant(id))!;
          const active=api.scene3d.getNavigationMode()===id;
          button.classList.toggle('on',active);button.setAttribute('aria-pressed',String(active));
          button.title=`${variant.label} (${CAMERA_TOOLS.indexOf(definition)+1}) · Drag to navigate · Active Camera edits the camera; Free View looks around`;
          button.setAttribute('aria-label',variant.label);
        }
      };
      const appendCameraTools=():void=>{
        for(const definition of CAMERA_TOOLS){
          const wrapper=document.createElement('span');wrapper.className='tl-group';
          const button=document.createElement('button');button.type='button';button.className='iconbtn tl';button.dataset.navigation=definition.id;
          button.innerHTML=api.ui.icon(definition.icon);button.addEventListener('click',()=>navigate(definition.id));
          const menu=()=>api.ui.menu(wrapper,definition.variants.map(variant=>({label:variant.label,on:api.scene3d.getNavigationVariant(definition.id)===variant.id,
            run:()=>{api.scene3d.setNavigationVariant(definition.id,variant.id);navigate(definition.id);}})));
          const more=document.createElement('button');more.type='button';more.className='iconbtn tl-more';more.title=`${definition.label} tools`;
          more.setAttribute('aria-label',more.title);more.setAttribute('aria-haspopup','menu');more.innerHTML=api.ui.icon('chevD');more.addEventListener('click',menu);
          button.addEventListener('keydown',event=>{if(event.key==='ArrowDown'){event.preventDefault();menu();}});
          wrapper.append(button,more);body.appendChild(wrapper);cameraButtons.push({button,id:definition.id});
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
        if(index===0)appendCameraTools();
      });

      toolListeners.add(syncTools);
      const navigationSubscription=api.scene3d.onNavigationChange(syncCamera);
      syncCamera();
      return () => {toolListeners.delete(syncTools);navigationSubscription.dispose();};
    }
  });
}
