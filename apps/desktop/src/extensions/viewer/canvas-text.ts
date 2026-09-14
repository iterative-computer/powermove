import type { PowermoveAPI, ViewerService } from 'powermove';

export interface CanvasTextState extends ViewerService {
  inner: HTMLElement;
  finishCanvasText?: ((cancel?: boolean) => void) | null;
  textSelection?: { layer: string; start: number; end: number } | null;
  requestOverlay?(): void;
}

export function editCanvasText(api: PowermoveAPI,V:CanvasTextState,layer:any,event?:MouseEvent,selectAll=false):void {
  if(layer.lock)return;V.finishCanvasText?.();api.selection.select([layer.id]);api.transport.pause();
  const time=api.transport.time(),d=api.anim.resolveContent(layer,time) as any,m=api.anim.worldMatrix(layer,time).map((v:number)=>v*V.shown) as [number,number,number,number,number,number],el=document.createElement('div');
  const offset=api.render.raster(layer,1,time)?.fontOffset as {x?:number;y?:number}|undefined;
  if(offset){const x=offset.x??0,y=offset.y??0;m[4]+=m[0]*x+m[2]*y;m[5]+=m[1]*x+m[3]*y;}
  el.contentEditable='plaintext-only';el.setAttribute('role','textbox');el.setAttribute('aria-label','Edit text on canvas');el.spellcheck=true;el.setAttribute('aria-multiline','true');
  el.className='canvas-text-editor';el.textContent=String(d.text||'');
  Object.assign(el.style,{position:'absolute',left:'0',top:'0',transformOrigin:'0 0',transform:`matrix(${m.join(',')})`,fontFamily:`"${String(d.font).replaceAll('"','')}"`,fontWeight:String(d.weight||500),fontStyle:d.italic?'italic':'normal',fontVariationSettings:Object.keys(d).filter(k=>k.startsWith('fontAxis.')&&Number.isFinite(Number(d[k]))).map(k=>`"${k.slice(9)}" ${Number(d[k])}`).join(',')||'normal',fontSize:`${d.size}px`,lineHeight:String(d.leading||1.15),letterSpacing:`${d.tracking||0}px`,color:d.color||'white',caretColor:d.color||'white',whiteSpace:d.boxWidth?'pre-wrap':'pre',width:d.boxWidth?`${d.boxWidth}px`:'max-content',minWidth:'1em',minHeight:'1em',outline:'1px solid #70b7ff',textAlign:d.align||'left',zIndex:'8',padding:'0',margin:'0',userSelect:'text',cursor:'text'});
  // Match the raster's first baseline (size * .82), rather than the
  // browser's font-dependent line-box baseline. This avoids a jump on entry.
  const probe=el.cloneNode(false) as HTMLDivElement;
  Object.assign(probe.style,{transform:'none',visibility:'hidden',width:'max-content'});
  probe.removeAttribute('contenteditable');probe.removeAttribute('role');probe.removeAttribute('aria-label');
  const marker=document.createElement('span');
  Object.assign(marker.style,{display:'inline-block',width:'0',height:'0',verticalAlign:'baseline'});
  probe.append(document.createTextNode('Mg'),marker);V.inner.appendChild(probe);
  const baseline=marker.getBoundingClientRect().bottom-probe.getBoundingClientRect().top;
  probe.remove();
  const place=()=>{
    const currentTime=api.transport.time();
    const matrix=api.anim.worldMatrix(layer,currentTime).map((v:number)=>v*V.shown) as [number,number,number,number,number,number];
    const fontOffset=api.render.raster(layer,1,currentTime)?.fontOffset as {x?:number;y?:number}|undefined;
    const x=fontOffset?.x||0,y=(fontOffset?.y||0)+Number(d.size)*.82-baseline;
    matrix[4]+=matrix[0]*x+matrix[2]*y;matrix[5]+=matrix[1]*x+matrix[3]*y;
    el.style.transform=`matrix(${matrix.join(',')})`;
    if(d.align==='center')el.style.transform+=' translateX(-50%)';
    else if(d.align==='right')el.style.transform+=' translateX(-100%)';
  };
  place();V.inner.appendChild(el);V.canvasTextEditing=layer.id;api.edit.begin('Edit source text',{origin:'canvas'});
  const capture=()=>{const s=window.getSelection();if(!s?.rangeCount || !el.contains(s.anchorNode) || !el.contains(s.focusNode))return;const r=s.getRangeAt(0),start=r.cloneRange(),end=r.cloneRange();start.selectNodeContents(el);start.setEnd(r.startContainer,r.startOffset);end.selectNodeContents(el);end.setEnd(r.endContainer,r.endOffset);V.textSelection={layer:layer.id,start:start.toString().length,end:end.toString().length};};
  const update=()=>{
    // Cutting/deleting everything leaves Chromium's caret placeholder BR.
    // Preserve actual line breaks, but never save that placeholder as text.
    const emptyPlaceholder=el.childNodes.length===1&&el.firstChild?.nodeName==='BR';
    const value=emptyPlaceholder?'':el.innerText.replace(/\r/g,'');
    api.edit.dispatch({type:'set_property',target:layer.id,path:'c.text',value,time:api.transport.time(),mode:'auto',preserveHandEdits:false});place();capture();
  };
  let done=false;const finish=(cancel=false)=>{if(done)return;done=true;capture();document.removeEventListener('selectionchange',capture);document.removeEventListener('pointerdown',outside,true);el.remove();V.canvasTextEditing=null;V.finishCanvasText=null;cancel?api.edit.cancel():api.edit.commit();api.transport.invalidate();V.requestOverlay?.();api.services.get<{refresh():void}>('inspector')?.refresh();};
  const outside=(e:PointerEvent)=>{
    if(el.contains(e.target as Node))return;
    finish();
    // Finish before the same pointer event reaches the viewer, so clicking
    // away selects normally instead of creating another text layer. A toolbar
    // click can still activate its explicitly requested tool afterward.
    const toolButton=(e.target as Element)?.closest?.('#toolbar button[data-tool]');
    const tools=api.services.get<{tool:string;setTool(tool:string):void}>('tool');
    if(tools?.tool==='text'&&!toolButton)tools.setTool('select');
  };
  V.finishCanvasText=finish;el.addEventListener('input',update);el.addEventListener('keydown',e=>{
    // Let keydowns bubble to the field-aware global keymap. It suppresses
    // editor commands for contenteditable targets and routes native editing
    // shortcuts (notably Electron paste) through the focused WebContents.
    if(e.key==='Escape'){e.preventDefault();finish(true);}else if(e.key==='Enter'&&(e.metaKey||e.ctrlKey)){e.preventDefault();finish();}
  });el.addEventListener('pointerdown',e=>e.stopPropagation());
  document.addEventListener('selectionchange',capture);document.addEventListener('pointerdown',outside,true);el.focus();
  const range=document.createRange();range.selectNodeContents(el);if(!selectAll)range.collapse(false);const selection=window.getSelection();selection?.removeAllRanges();selection?.addRange(range);
  if(event){const caret=(document as any).caretRangeFromPoint?.(event.clientX,event.clientY);if(caret&&el.contains(caret.startContainer)){selection?.removeAllRanges();selection?.addRange(caret);}}
  capture();api.transport.invalidate('render');V.requestOverlay?.();
}
