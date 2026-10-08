import * as THREE from 'three';
import {AXIS_COLORS,type Point} from './modal';
import type {ViewAxis} from './viewport';
import type {GizmoGeometry,GizmoHandle} from './handles';

/*
 * The 3D viewer's screen-space chrome, shown only for compositions with 3D
 * layers and laid over the preview (it takes no layout space): a small
 * navigation cluster (axis ball, camera view, view menu), the selection
 * gizmo drawn like a 2D selection box, and a readout while dragging.
 */

export type ChromeState={
  visible:boolean;
  rotation:THREE.Quaternion;
  axis:ViewAxis|null;
  camera:boolean;
  /** Selection ink, matching 2D selection boxes. */
  ink:string;
  gizmo:GizmoGeometry|null;
  hover:GizmoHandle|null;
  readout:{text:string;at:Point}|null;
};
export type ChromeActions={
  drag(event:PointerEvent,kind:'orbit'|'pan'|'dolly'):void;
  toggleCamera():void;
  align(axis:ViewAxis):void;
  menu(anchor:HTMLElement):void;
};

const SVG='http://www.w3.org/2000/svg';
const svg=<K extends keyof SVGElementTagNameMap>(name:K,attributes:Record<string,string|number>={}):SVGElementTagNameMap[K]=>{
  const element=document.createElementNS(SVG,name);for(const [key,value] of Object.entries(attributes))element.setAttribute(key,String(value));return element;
};
const BALL=56,RADIUS=19;
const BUBBLES:{axis:'x'|'y'|'z';sign:1|-1;view:ViewAxis;label:string}[]=[
  {axis:'x',sign:1,view:'right',label:'X'},{axis:'x',sign:-1,view:'left',label:''},{axis:'y',sign:1,view:'bottom',label:'Y'},
  {axis:'y',sign:-1,view:'top',label:''},{axis:'z',sign:1,view:'back',label:'Z'},{axis:'z',sign:-1,view:'front',label:''}
];
const ICON={
  camera:'<rect x="3" y="7" width="13" height="10" rx="2" fill="none" stroke="currentColor" stroke-width="1.7"/><path d="m16 10.5 5-3v9l-5-3" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"/>',
  more:'<circle cx="6" cy="12" r="1.6" fill="currentColor"/><circle cx="12" cy="12" r="1.6" fill="currentColor"/><circle cx="18" cy="12" r="1.6" fill="currentColor"/>'
};
const STYLE=`
[data-viewport-chrome]{position:absolute;z-index:5;user-select:none;-webkit-user-select:none}
[data-viewport-chrome][hidden]{display:none!important}
.vp-nav{right:10px;top:10px;display:flex;flex-direction:column;align-items:center;gap:4px;padding:4px;border-radius:var(--r-md,10px);background:color-mix(in srgb,var(--bg-panel,#1e1e1e) 78%,transparent)}
.vp-ball{display:block;border-radius:50%;cursor:grab;touch-action:none}
.vp-ball:hover{background:color-mix(in srgb,var(--tx,#fff) 8%,transparent)}
.vp-ball text{font:700 8.5px var(--f-ui,system-ui);fill:#141414;pointer-events:none}
.vp-ball .vp-bubble{cursor:pointer}
.vp-nav button{all:unset;box-sizing:border-box;width:28px;height:24px;border-radius:var(--r-sm,6px);display:grid;place-items:center;color:var(--tx-2,#ccc);cursor:pointer}
.vp-nav button:hover{background:color-mix(in srgb,var(--tx,#fff) 8%,transparent);color:var(--tx,#fff)}
.vp-nav button[aria-pressed="true"]{color:var(--accent,#ff7a33)}
.vp-nav button:focus-visible{outline:2px solid var(--accent,#ff7a33);outline-offset:-2px}
.vp-nav button svg{width:17px;height:17px}
.vp-readout{padding:3px 7px;border-radius:var(--r-sm,6px);background:var(--bg-float,rgba(24,24,24,.92));color:var(--tx,#fff);font:var(--fs-xs,11px) var(--f-ui,system-ui);font-variant-numeric:tabular-nums;white-space:pre;pointer-events:none}
.vp-gizmo{position:absolute;inset:0;width:100%;height:100%;z-index:5;pointer-events:none;overflow:visible}
`;

export class ViewportChrome {
  private elements:HTMLElement[]=[];
  private nav:HTMLDivElement;
  private ballContent:SVGGElement;
  private cameraButton:HTMLButtonElement;
  private readout:HTMLDivElement;
  private gizmo:SVGSVGElement;
  private style:HTMLStyleElement;
  private offs:(()=>void)[]=[];

  constructor(stage:HTMLElement,private actions:ChromeActions){
    this.style=document.createElement('style');this.style.textContent=STYLE;stage.append(this.style);
    const mark=<T extends HTMLElement>(element:T,className:string)=>{element.dataset.viewportChrome='';element.className=className;element.hidden=true;stage.append(element);this.elements.push(element);return element;};
    this.nav=mark(document.createElement('div'),'vp-nav');
    const ball=svg('svg',{width:BALL,height:BALL,viewBox:`0 0 ${BALL} ${BALL}`,class:'vp-ball',role:'img','aria-label':'View orientation: drag to orbit, click an axis to look along it'});
    this.ballContent=svg('g');ball.append(this.ballContent);
    const button=(label:string,icon:string)=>{const element=document.createElement('button');element.type='button';element.setAttribute('aria-label',label);element.title=label;element.innerHTML=`<svg viewBox="0 0 24 24" aria-hidden="true">${icon}</svg>`;return element;};
    this.cameraButton=button('Camera view','');this.cameraButton.innerHTML=`<svg viewBox="0 0 24 24" aria-hidden="true">${ICON.camera}</svg>`;
    const more=button('View options',ICON.more);more.setAttribute('aria-haspopup','menu');
    this.nav.append(ball,this.cameraButton,more);
    this.readout=mark(document.createElement('div'),'vp-readout');
    this.gizmo=svg('svg',{class:'vp-gizmo','aria-hidden':'true'});this.gizmo.dataset.viewportChrome='';this.gizmo.dataset.sceneGizmo='';stage.append(this.gizmo);

    const listen=(target:EventTarget,type:string,handler:any)=>{target.addEventListener(type,handler);this.offs.push(()=>target.removeEventListener(type,handler));};
    listen(ball,'pointerdown',(event:PointerEvent)=>{
      if(event.button!==0)return;event.preventDefault();event.stopPropagation();
      const bubble=(event.target as Element).closest?.('[data-view]')?.getAttribute('data-view') as ViewAxis|null,start={x:event.clientX,y:event.clientY};
      // Listen before the orbit drag does: it stops the pointerup it handles.
      const up=(e:PointerEvent)=>{window.removeEventListener('pointerup',up,true);if(bubble&&Math.hypot(e.clientX-start.x,e.clientY-start.y)<4)queueMicrotask(()=>this.actions.align(bubble));};
      window.addEventListener('pointerup',up,true);
      this.actions.drag(event,'orbit');
    });
    listen(this.cameraButton,'click',()=>this.actions.toggleCamera());
    listen(more,'click',()=>this.actions.menu(more));
    for(const element of [...this.elements,this.gizmo])listen(element,'contextmenu',(event:Event)=>event.preventDefault());
  }

  update(state:ChromeState):void {
    const visible=state.visible;
    this.nav.hidden=!visible;this.gizmo.style.display=visible?'':'none';
    if(!visible){this.readout.hidden=true;return;}
    this.drawBall(state);
    this.cameraButton.setAttribute('aria-pressed',String(state.camera));
    this.cameraButton.title=state.camera?'Camera view (click for a free view)':'Free view (click to return to the camera)';
    this.readout.hidden=!state.readout;
    if(state.readout){this.readout.textContent=state.readout.text;this.readout.style.left=`${state.readout.at.x+14}px`;this.readout.style.top=`${state.readout.at.y+14}px`;}
    this.drawGizmo(state);
  }
  private drawBall(state:ChromeState):void {
    const inverse=state.rotation.clone().invert(),c=BALL/2;
    // Composition axes: x right, y down, z away (scene y is up, z toward the viewer).
    const scene={x:new THREE.Vector3(1,0,0),y:new THREE.Vector3(0,-1,0),z:new THREE.Vector3(0,0,-1)};
    const items=BUBBLES.map(bubble=>{const v=scene[bubble.axis].clone().multiplyScalar(bubble.sign).applyQuaternion(inverse);return {...bubble,x:c+v.x*RADIUS,y:c-v.y*RADIUS,depth:v.z};}).sort((a,b)=>a.depth-b.depth);
    const nodes:SVGElement[]=[];
    for(const item of items){
      const color=AXIS_COLORS[item.axis],positive=item.sign>0;
      if(positive)nodes.push(svg('line',{x1:c,y1:c,x2:item.x,y2:item.y,stroke:color,'stroke-width':1.6,'stroke-linecap':'round'}));
      const group=svg('g',{class:'vp-bubble','data-view':item.view});
      const title=svg('title');title.textContent=`Look along ${item.sign>0?'':'−'}${item.axis.toUpperCase()}`;group.append(title);
      group.append(svg('circle',positive?{cx:item.x,cy:item.y,r:6,fill:color}:{cx:item.x,cy:item.y,r:4.5,fill:color,'fill-opacity':.35}));
      if(item.label){const text=svg('text',{x:item.x,y:item.y+3,'text-anchor':'middle'});text.textContent=item.label;group.append(text);}
      nodes.push(group);
    }
    this.ballContent.replaceChildren(...nodes);
  }
  /** The same marks as a 2D selection: ink outline, white square handles, a rotate stem and the anchor. */
  private drawGizmo(state:ChromeState):void {
    const g=state.gizmo,nodes:SVGElement[]=[];
    if(g){
      const ink=state.ink,hot=state.hover;
      const handle=(p:Point,attrs:Record<string,string|number>={})=>svg('rect',{x:p.x-5,y:p.y-5,width:10,height:10,fill:'#fff',stroke:'rgba(0,0,0,.45)','stroke-width':1,...attrs});
      if(g.box){
        nodes.push(svg('polygon',{points:g.box.map(p=>`${p.x},${p.y}`).join(' '),fill:'none',stroke:ink,'stroke-width':1.4}));
        if(g.stem){
          nodes.push(svg('line',{x1:g.stem.from.x,y1:g.stem.from.y,x2:g.stem.to.x,y2:g.stem.to.y,stroke:ink,'stroke-width':1.4}));
          nodes.push(svg('circle',{'data-handle':'rotate',cx:g.stem.to.x,cy:g.stem.to.y,r:hot?.kind==='rotate'?5:4,fill:'#fff',stroke:'rgba(0,0,0,.45)','stroke-width':1}));
        }
        g.box.forEach((p,index)=>nodes.push(handle(p,{'data-handle':'corner',...(hot?.kind==='corner'&&hot.index===index?{fill:ink}:{})})));
      }
      for(const ring of g.rings)nodes.push(svg('polyline',{points:ring.points.map(p=>`${p.x},${p.y}`).join(' '),fill:'none',stroke:ring.color,'stroke-width':hot?.kind==='ring'&&hot.axis===ring.axis?2.6:1.5,'stroke-opacity':.95}));
      for(const axis of g.axes){
        const active=hot?.kind==='axis'&&hot.axis===axis.axis;
        nodes.push(svg('line',{x1:g.pivot.x,y1:g.pivot.y,x2:axis.tip.x,y2:axis.tip.y,stroke:axis.color,'stroke-width':active?2.4:1.6,'stroke-linecap':'round'}));
        nodes.push(svg('rect',{'data-handle':`axis-${axis.axis}`,x:axis.tip.x-4.5,y:axis.tip.y-4.5,width:9,height:9,fill:active?axis.color:'#fff',stroke:axis.color,'stroke-width':1.6}));
      }
      for(const aim of g.aims){
        nodes.push(svg('line',{x1:aim.from.x,y1:aim.from.y,x2:aim.at.x,y2:aim.at.y,stroke:ink,'stroke-width':1,'stroke-dasharray':'4 3',opacity:.8}));
        nodes.push(svg('circle',{'data-handle':'aim',cx:aim.at.x,cy:aim.at.y,r:hot?.kind==='aim'&&hot.id===aim.id?6:5,fill:'#fff',stroke:ink,'stroke-width':1.4}));
      }
      // The anchor, exactly like a 2D layer's.
      const a=g.pivot,anchor=svg('g');
      for(const [stroke,width] of [['rgba(0,0,0,.8)',3],[ink,1]] as const)
        anchor.append(svg('circle',{cx:a.x,cy:a.y,r:5,fill:'none',stroke,'stroke-width':width}),svg('path',{d:`M${a.x-8} ${a.y}H${a.x+8}M${a.x} ${a.y-8}V${a.y+8}`,stroke,'stroke-width':width}));
      nodes.push(anchor);
    }
    this.gizmo.replaceChildren(...nodes);
  }
  dispose():void {
    for(const off of this.offs.splice(0))off();
    for(const element of this.elements)element.remove();this.gizmo.remove();this.style.remove();
  }
}
