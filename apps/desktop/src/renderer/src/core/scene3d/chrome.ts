import * as THREE from 'three';
import {AXIS_COLORS,type Point} from './modal';
import type {ViewAxis} from './viewport';

/*
 * The viewport's screen-space chrome, shown only for compositions with 3D
 * layers: Blender's navigation gizmo (axis ball plus zoom, pan, camera and
 * projection buttons), the view name in the top-left corner, the 3D cursor,
 * and the operator header and key hints while a modal transform runs.
 */

export type ChromeState={
  visible:boolean;
  rotation:THREE.Quaternion;
  axis:ViewAxis|null;
  camera:boolean;
  orthographic:boolean;
  label:string;
  detail:string;
  leftInset:number;
  rightInset:number;
  cursor:Point|null;
  /** Zoom Border's rectangle while it is dragged. */
  box?:{x:number;y:number;width:number;height:number}|null;
  modal:{header:string;hints:[string,string][];link:{from:Point;to:Point}|null}|null;
};
export type ChromeActions={
  drag(event:PointerEvent,kind:'orbit'|'pan'|'dolly'):void;
  toggleCamera():void;
  toggleProjection():void;
  align(axis:ViewAxis):void;
};

const SVG='http://www.w3.org/2000/svg';
const svg=<K extends keyof SVGElementTagNameMap>(name:K,attributes:Record<string,string|number>={}):SVGElementTagNameMap[K]=>{
  const element=document.createElementNS(SVG,name);for(const [key,value] of Object.entries(attributes))element.setAttribute(key,String(value));return element;
};
const BALL=92,RADIUS=31;
const BUBBLES:{axis:'x'|'y'|'z';sign:1|-1;view:ViewAxis}[]=[
  {axis:'x',sign:1,view:'right'},{axis:'x',sign:-1,view:'left'},{axis:'y',sign:1,view:'top'},
  {axis:'y',sign:-1,view:'bottom'},{axis:'z',sign:1,view:'front'},{axis:'z',sign:-1,view:'back'}
];
const ICONS:Record<string,string>={
  dolly:'<circle cx="10.5" cy="10.5" r="5.5" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="M14.5 14.5 19 19" stroke="currentColor" stroke-width="2" stroke-linecap="round"/><path d="M8 10.5h5M10.5 8v5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>',
  pan:'<path d="M12 3v18M3 12h18" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/><path d="m9 6 3-3 3 3M9 18l3 3 3-3M6 9l-3 3 3 3M18 9l3 3-3 3" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>',
  camera:'<rect x="3" y="7" width="13" height="10" rx="2" fill="none" stroke="currentColor" stroke-width="1.7"/><path d="m16 10.5 5-3v9l-5-3" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"/>',
  perspective:'<path d="M7 5h10l4 14H3z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"/><path d="M12 5v14M5 12h14" stroke="currentColor" stroke-width="1.3"/>',
  orthographic:'<rect x="4" y="4" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.6"/><path d="M12 4v16M4 12h16" stroke="currentColor" stroke-width="1.3"/>'
};
const STYLE=`
[data-viewport-chrome]{position:absolute;z-index:5;font:var(--fs-sm,12px)/1.35 var(--f-ui,system-ui);user-select:none;-webkit-user-select:none}
[data-viewport-chrome][hidden]{display:none!important}
.vp-nav{right:10px;top:10px;display:flex;flex-direction:column;align-items:center;gap:6px;pointer-events:none}
.vp-ball{pointer-events:auto;border-radius:50%;cursor:grab;touch-action:none}
.vp-ball .vp-backdrop{fill:rgba(255,255,255,0);transition:fill 120ms ease}
.vp-ball:hover .vp-backdrop{fill:rgba(255,255,255,.1)}
.vp-ball text{font:700 10.5px var(--f-ui,system-ui);fill:#141414;pointer-events:none}
.vp-ball .vp-bubble{cursor:pointer}
.vp-ball .vp-bubble:hover{filter:brightness(1.25)}
.vp-buttons{display:flex;flex-direction:column;gap:5px;pointer-events:auto}
.vp-buttons button{all:unset;box-sizing:border-box;width:28px;height:28px;border-radius:50%;display:grid;place-items:center;color:rgba(255,255,255,.88);background:rgba(36,36,36,.72);cursor:pointer;touch-action:none}
.vp-buttons button:hover{background:rgba(70,70,70,.85)}
.vp-buttons button[aria-pressed="true"]{background:#4772b3;color:#fff}
.vp-buttons button:focus-visible{outline:2px solid var(--accent,#4772b3);outline-offset:1px}
.vp-buttons svg{width:17px;height:17px}
.vp-label{left:12px;top:10px;color:rgba(255,255,255,.9);text-shadow:0 1px 2px rgba(0,0,0,.9),0 0 1px rgba(0,0,0,.9);pointer-events:none;white-space:nowrap}
.vp-label span{display:block}
.vp-label span+span{opacity:.8}
.vp-op-header{left:50%;top:10px;transform:translateX(-50%);padding:5px 10px;border-radius:6px;background:rgba(24,24,24,.9);color:#fff;font-variant-numeric:tabular-nums;white-space:pre;pointer-events:none}
.vp-op-hints{left:50%;bottom:10px;transform:translateX(-50%);display:flex;flex-wrap:wrap;justify-content:center;max-width:calc(100% - 300px);gap:4px 12px;align-items:center;padding:5px 10px;border-radius:6px;background:rgba(24,24,24,.86);color:rgba(255,255,255,.82);white-space:nowrap;pointer-events:none}
.vp-op-hints kbd{font:inherit;font-size:11px;padding:1px 5px;margin-right:5px;border-radius:4px;background:rgba(255,255,255,.14);color:#fff}
.vp-lines{position:absolute;inset:0;width:100%;height:100%;z-index:5;pointer-events:none;overflow:visible}
`;

export class ViewportChrome {
  private root:HTMLElement[]=[];
  private nav:HTMLDivElement;
  private ball:SVGSVGElement;
  private ballContent:SVGGElement;
  private buttons:Record<'dolly'|'pan'|'camera'|'projection',HTMLButtonElement>;
  private label:HTMLDivElement;
  private header:HTMLDivElement;
  private hints:HTMLDivElement;
  private lines:SVGSVGElement;
  private link:SVGLineElement;
  private cursor:SVGGElement;
  private border:SVGRectElement;
  private style:HTMLStyleElement;
  private state:ChromeState|null=null;
  private offs:(()=>void)[]=[];

  constructor(private stage:HTMLElement,private actions:ChromeActions){
    this.style=document.createElement('style');this.style.textContent=STYLE;stage.append(this.style);
    const mark=<T extends HTMLElement>(element:T,className:string)=>{element.dataset.viewportChrome='';element.className=className;element.hidden=true;stage.append(element);this.root.push(element);return element;};
    this.nav=mark(document.createElement('div'),'vp-nav');
    this.ball=svg('svg',{width:BALL,height:BALL,viewBox:`0 0 ${BALL} ${BALL}`,class:'vp-ball',role:'img','aria-label':'Navigation gizmo: drag to orbit, click an axis to align the view'});
    this.ball.append(svg('circle',{cx:BALL/2,cy:BALL/2,r:BALL/2-1,class:'vp-backdrop'}));
    this.ballContent=svg('g');this.ball.append(this.ballContent);
    const buttons=document.createElement('div');buttons.className='vp-buttons';
    const button=(key:string,label:string,icon:string)=>{
      const element=document.createElement('button');element.type='button';element.setAttribute('aria-label',label);element.title=label;
      element.innerHTML=`<svg viewBox="0 0 24 24" aria-hidden="true">${ICONS[icon]}</svg>`;element.dataset.navButton=key;buttons.append(element);return element;
    };
    this.buttons={
      dolly:button('dolly','Zoom (drag, Ctrl+MMB)','dolly'),
      pan:button('pan','Move view (drag, Shift+MMB)','pan'),
      camera:button('camera','Toggle camera view (Numpad 0)','camera'),
      projection:button('projection','Switch perspective/orthographic (Numpad 5)','perspective')
    };
    this.nav.append(this.ball,buttons);
    this.label=mark(document.createElement('div'),'vp-label');
    this.header=mark(document.createElement('div'),'vp-op-header');this.header.setAttribute('role','status');
    this.hints=mark(document.createElement('div'),'vp-op-hints');
    this.lines=svg('svg',{class:'vp-lines','aria-hidden':'true'});this.lines.dataset.viewportChrome='';
    this.link=svg('line',{stroke:'#fff','stroke-width':1.2,'stroke-dasharray':'5 4',opacity:.85});
    this.cursor=svg('g',{'data-scene-cursor':''});
    this.cursor.append(svg('circle',{r:9,fill:'none',stroke:'#fff','stroke-width':2}),svg('circle',{r:9,fill:'none',stroke:'#ff2b2b','stroke-width':2,'stroke-dasharray':'4.7 4.7'}));
    for(const [x1,y1,x2,y2] of [[0,-15,0,-5],[0,5,0,15],[-15,0,-5,0],[5,0,15,0]])this.cursor.append(svg('line',{x1:x1!,y1:y1!,x2:x2!,y2:y2!,stroke:'#111','stroke-width':3}),svg('line',{x1:x1!,y1:y1!,x2:x2!,y2:y2!,stroke:'#fff','stroke-width':1.2}));
    this.border=svg('rect',{fill:'rgba(255,255,255,.06)',stroke:'#fff','stroke-width':1,'stroke-dasharray':'4 3'});
    this.lines.append(this.link,this.cursor,this.border);stage.append(this.lines);

    const listen=(target:EventTarget,type:string,handler:any)=>{target.addEventListener(type,handler);this.offs.push(()=>target.removeEventListener(type,handler));};
    listen(this.ball,'pointerdown',(event:PointerEvent)=>{
      if(event.button!==0)return;event.preventDefault();event.stopPropagation();
      const bubble=(event.target as Element).closest?.('[data-view]')?.getAttribute('data-view') as ViewAxis|null;
      const start={x:event.clientX,y:event.clientY};
      // Listen before the orbit drag does: it stops the pointerup it handles.
      const up=(e:PointerEvent)=>{
        window.removeEventListener('pointerup',up,true);
        if(bubble&&Math.hypot(e.clientX-start.x,e.clientY-start.y)<4)queueMicrotask(()=>this.actions.align(bubble));
      };
      window.addEventListener('pointerup',up,true);
      this.actions.drag(event,'orbit');
    });
    for(const kind of ['dolly','pan'] as const)listen(this.buttons[kind],'pointerdown',(event:PointerEvent)=>{if(event.button!==0)return;event.preventDefault();event.stopPropagation();this.actions.drag(event,kind);});
    listen(this.buttons.camera,'click',()=>this.actions.toggleCamera());
    listen(this.buttons.projection,'click',()=>this.actions.toggleProjection());
    for(const element of [...this.root,this.lines])listen(element,'contextmenu',(event:Event)=>event.preventDefault());
  }

  update(state:ChromeState):void {
    this.state=state;
    const visible=state.visible;
    this.nav.hidden=!visible;this.label.hidden=!visible;this.lines.style.display=visible?'':'none';
    if(!visible){this.header.hidden=true;this.hints.hidden=true;return;}
    this.drawBall(state);
    this.buttons.camera.setAttribute('aria-pressed',String(state.camera));
    this.buttons.projection.querySelector('svg')!.innerHTML=ICONS[state.orthographic?'orthographic':'perspective']!;
    this.label.style.left=`${12+state.leftInset}px`;this.nav.style.right=`${10+state.rightInset}px`;
    // Centre operator text in the area the toolbar and sidebar leave free.
    this.header.style.left=this.hints.style.left=`calc(50% + ${(state.leftInset-state.rightInset)/2}px)`;
    this.label.replaceChildren(...[state.label,state.detail].filter(Boolean).map(text=>{const span=document.createElement('span');span.textContent=text;return span;}));
    if(state.modal){
      this.header.hidden=false;this.header.textContent=state.modal.header;
      this.hints.hidden=false;
      this.hints.replaceChildren(...state.modal.hints.map(([key,label])=>{const item=document.createElement('span');const kbd=document.createElement('kbd');kbd.textContent=key;item.append(kbd,label);return item;}));
    }else{this.header.hidden=true;this.hints.hidden=true;}
    const link=state.modal?.link;
    this.link.style.display=link?'':'none';
    if(link){this.link.setAttribute('x1',String(link.from.x));this.link.setAttribute('y1',String(link.from.y));this.link.setAttribute('x2',String(link.to.x));this.link.setAttribute('y2',String(link.to.y));}
    this.border.style.display=state.box?'':'none';
    if(state.box)for(const key of ['x','y','width','height'] as const)this.border.setAttribute(key,String(state.box[key]));
    this.cursor.style.display=state.cursor?'':'none';
    if(state.cursor)this.cursor.setAttribute('transform',`translate(${state.cursor.x} ${state.cursor.y})`);
  }
  private drawBall(state:ChromeState):void {
    const inverse=state.rotation.clone().invert(),c=BALL/2;
    const items=BUBBLES.map(bubble=>{
      const v=new THREE.Vector3(bubble.axis==='x'?bubble.sign:0,bubble.axis==='y'?bubble.sign:0,bubble.axis==='z'?bubble.sign:0).applyQuaternion(inverse);
      return {...bubble,x:c+v.x*RADIUS,y:c-v.y*RADIUS,depth:v.z};
    }).sort((a,b)=>a.depth-b.depth);
    const nodes:SVGElement[]=[];
    for(const item of items){
      const color=AXIS_COLORS[item.axis],positive=item.sign>0;
      if(positive)nodes.push(svg('line',{x1:c,y1:c,x2:item.x,y2:item.y,stroke:color,'stroke-width':2.2,'stroke-linecap':'round',opacity:item.depth<-.2?.55:1}));
      const group=svg('g',{class:'vp-bubble','data-view':item.view});
      const title=svg('title');title.textContent=`${item.view[0]!.toUpperCase()+item.view.slice(1)} view (${item.sign>0?'':'-'}${item.axis.toUpperCase()})`;group.append(title);
      group.append(svg('circle',positive?{cx:item.x,cy:item.y,r:8.5,fill:color}:{cx:item.x,cy:item.y,r:7.5,fill:color,'fill-opacity':.32,stroke:color,'stroke-width':1.4}));
      if(positive){const text=svg('text',{x:item.x,y:item.y+3.6,'text-anchor':'middle'});text.textContent=item.axis.toUpperCase();group.append(text);}
      nodes.push(group);
    }
    this.ballContent.replaceChildren(...nodes);
  }
  dispose():void {
    for(const off of this.offs.splice(0))off();
    for(const element of this.root)element.remove();this.lines.remove();this.style.remove();
  }
}
