/*
 * Blender pie menus. Items fill Blender's slot order: West, East, South,
 * North, North-west, North-east, South-west, South-east. Tap the key and the
 * pie stays open for a click; hold it, point toward an item and release to
 * choose it. Escape, a right click or a click in the middle closes it.
 */

export type PieItem={label:string;icon?:string;on?:boolean;disabled?:boolean;run:()=>unknown}|null;
const SLOTS=[180,0,270,90,135,45,225,315];
const RADIUS=118,DEAD_ZONE=26,HOLD_MS=260;
const STYLE=`
.pm-pie{position:fixed;inset:0;z-index:410;cursor:default;user-select:none;-webkit-user-select:none}
.pm-pie-title{position:absolute;transform:translate(-50%,-50%);padding:3px 9px;border-radius:5px;background:rgba(24,24,24,.92);color:rgba(255,255,255,.75);font:600 11px var(--f-ui,system-ui);white-space:nowrap;pointer-events:none}
.pm-pie-ring{position:absolute;width:44px;height:44px;margin:-22px 0 0 -22px;border-radius:50%;border:5px solid rgba(24,24,24,.86);box-sizing:border-box;pointer-events:none}
.pm-pie-pointer{position:absolute;left:50%;top:50%;width:10px;height:10px;margin:-5px 0 0 -5px;border-radius:50%;background:#4772b3;opacity:0;transition:opacity 80ms}
.pm-pie button{all:unset;position:absolute;box-sizing:border-box;display:flex;align-items:center;gap:7px;height:28px;padding:0 12px;border-radius:6px;background:rgba(40,40,40,.96);color:#e6e6e6;font:12px var(--f-ui,system-ui);white-space:nowrap;box-shadow:0 2px 8px rgba(0,0,0,.35);cursor:pointer}
.pm-pie button[data-hot]{background:#4772b3;color:#fff}
.pm-pie button[aria-checked="true"]::after{content:'';width:6px;height:6px;border-radius:50%;background:currentColor;margin-left:2px}
.pm-pie button:disabled{opacity:.45;cursor:default}
.pm-pie button svg{flex:none}
`;
let open:{close():void}|null=null;

export function openPie(options:{title:string;items:PieItem[];at:{x:number;y:number};releaseCode?:string|null}):void {
  open?.close();
  const style=document.createElement('style');style.textContent=STYLE;
  const root=document.createElement('div');root.className='pm-pie';root.setAttribute('role','menu');root.setAttribute('aria-label',options.title);
  const x=Math.min(window.innerWidth-RADIUS-90,Math.max(RADIUS+90,options.at.x)),y=Math.min(window.innerHeight-RADIUS-30,Math.max(RADIUS+30,options.at.y));
  const ring=document.createElement('div');ring.className='pm-pie-ring';ring.style.left=`${x}px`;ring.style.top=`${y}px`;
  const dot=document.createElement('div');dot.className='pm-pie-pointer';ring.append(dot);
  const title=document.createElement('div');title.className='pm-pie-title';title.textContent=options.title;title.style.left=`${x}px`;title.style.top=`${y-RADIUS-34}px`;
  root.append(ring,title);
  const buttons:{button:HTMLButtonElement;angle:number;item:Exclude<PieItem,null>}[]=[];
  options.items.slice(0,8).forEach((item,index)=>{
    if(!item)return;
    const angle=SLOTS[index]!,rad=angle*Math.PI/180,bx=x+Math.cos(rad)*RADIUS,by=y-Math.sin(rad)*RADIUS;
    const button=document.createElement('button');button.type='button';button.setAttribute('role','menuitemradio');button.setAttribute('aria-checked',String(!!item.on));
    button.disabled=!!item.disabled;button.innerHTML=item.icon||'';button.append(item.label);
    root.append(button);
    // Anchor each button on the side facing the centre, like Blender.
    const width=button.offsetWidth||Math.max(80,item.label.length*7+30),cos=Math.cos(rad);
    button.style.left=`${bx-(Math.abs(cos)<.3?width/2:cos<0?width:0)}px`;button.style.top=`${by-14}px`;
    buttons.push({button,angle,item});
  });
  document.body.append(style,root);
  for(const entry of buttons){const width=entry.button.offsetWidth,cos=Math.cos(entry.angle*Math.PI/180),bx=x+cos*RADIUS;entry.button.style.left=`${bx-(Math.abs(cos)<.3?width/2:cos<0?width:0)}px`;}
  const opened=performance.now();let hot:typeof buttons[number]|null=null,closed=false;
  const setHot=(next:typeof hot)=>{if(hot===next)return;hot?.button.removeAttribute('data-hot');hot=next;hot?.button.setAttribute('data-hot','');dot.style.opacity=hot?'1':'0';};
  const choose=(entry:typeof hot)=>{if(!entry||entry.item.disabled)return;close();try{entry.item.run();}catch(error){console.error('[viewer] pie action failed',error);}};
  const move=(event:PointerEvent)=>{
    const dx=event.clientX-x,dy=y-event.clientY,distance=Math.hypot(dx,dy);
    if(distance<DEAD_ZONE){setHot(null);return;}
    const angle=(Math.atan2(dy,dx)*180/Math.PI+360)%360;
    let best:typeof hot=null,score=Infinity;
    for(const entry of buttons){if(entry.item.disabled)continue;const delta=Math.abs(((angle-entry.angle+540)%360)-180);if(delta<score){score=delta;best=entry;}}
    setHot(best);
    dot.style.transform=`translate(${Math.cos(Math.atan2(dy,dx))*10}px,${-Math.sin(Math.atan2(dy,dx))*10}px)`;
  };
  const down=(event:PointerEvent)=>{
    event.preventDefault();event.stopPropagation();
    if(event.button===2){close();return;}
    const target=buttons.find(entry=>entry.button.contains(event.target as Node));
    if(target){choose(target);return;}
    if(hot)choose(hot);else close();
  };
  const key=(event:KeyboardEvent)=>{
    if(event.key==='Escape'){event.preventDefault();event.stopImmediatePropagation();close();return;}
    if(event.key==='Enter'&&hot){event.preventDefault();event.stopImmediatePropagation();choose(hot);return;}
    if(event.code===options.releaseCode&&event.repeat){event.preventDefault();event.stopImmediatePropagation();}
  };
  const keyup=(event:KeyboardEvent)=>{
    if(!options.releaseCode||event.code!==options.releaseCode)return;
    // Held past a tap: release chooses the item under the pointer direction.
    if(performance.now()-opened>HOLD_MS){if(hot)choose(hot);else close();}
  };
  const blur=()=>close();
  root.addEventListener('pointermove',move);root.addEventListener('pointerdown',down);root.addEventListener('contextmenu',e=>e.preventDefault());
  window.addEventListener('keydown',key,true);window.addEventListener('keyup',keyup,true);window.addEventListener('blur',blur);
  function close(){
    if(closed)return;closed=true;if(open===handle)open=null;
    window.removeEventListener('keydown',key,true);window.removeEventListener('keyup',keyup,true);window.removeEventListener('blur',blur);
    root.remove();style.remove();
  }
  const handle={close};open=handle;
}
export const closePie=()=>open?.close();
