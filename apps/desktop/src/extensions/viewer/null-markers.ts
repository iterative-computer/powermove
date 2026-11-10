import type { PowermoveAPI, Point, Scene3DViewerRect } from 'powermove';

export type NullMarker={id:string;x:number;y:number;selected:boolean;corners?:Point[];scene?:boolean};
export function collectNullMarkers(api:PowermoveAPI,rect:Scene3DViewerRect,time:number,nativePivot:(layer:any)=>Point,sceneControl:(layer:any)=>boolean,nativeCorners:(layer:any)=>Point[]):NullMarker[]{
  const selected=new Set(api.selection.layers()),comp=api.model.curComp(),out:NullMarker[]=[];
  const toViewer=(point:Point)=>({x:rect.x+point.x/comp.w*rect.width,y:rect.y+point.y/comp.h*rect.height});
  for(const layer of comp.layers){
    if(layer.type!=='null'||!api.anim.active(layer,time))continue;
    let point:Point|null,corners:Point[]|undefined;const scene=sceneControl(layer);
    if(scene){const state=api.scene3d.nullViewerState?.(layer.id,time,rect);point=state?.viewerPosition??api.scene3d.nullViewerPosition?.(layer.id,time,rect)??null;corners=state?.corners;}
    else {point=toViewer(nativePivot(layer));corners=nativeCorners(layer).map(toViewer);}
    if(point&&Number.isFinite(point.x)&&Number.isFinite(point.y)&&(!corners||corners.every(p=>Number.isFinite(p.x)&&Number.isFinite(p.y))))out.push({id:layer.id,...point,selected:selected.has(layer.id),...(corners?{corners}:{}),...(scene?{scene:true}:{})});
  }
  return out;
}
export function drawNullMarkers(ctx:CanvasRenderingContext2D,markers:NullMarker[],ink:string,hoverId:string|null=null):void {
  for(const marker of markers){
    // Selected native nulls already draw this exact geometry with their handles.
    if(marker.corners&&marker.selected&&!marker.scene)continue;
    ctx.save();
    ctx.globalAlpha=marker.selected||marker.id===hoverId?1:.35;
    if(marker.corners?.length){
      ctx.beginPath();ctx.moveTo(marker.corners[0]!.x,marker.corners[0]!.y);
      for(const point of marker.corners.slice(1))ctx.lineTo(point.x,point.y);
      ctx.closePath();ctx.strokeStyle=ink;ctx.lineWidth=1.4;ctx.stroke();

    }
    if(marker.corners&&!marker.selected){ctx.restore();continue;}
    ctx.translate(marker.x,marker.y);ctx.beginPath();
    if(marker.corners){
      ctx.arc(0,0,5,0,Math.PI*2);ctx.moveTo(-8,0);ctx.lineTo(8,0);ctx.moveTo(0,-8);ctx.lineTo(0,8);
    }else{
      ctx.rect(-7,-7,14,14);
      if(marker.selected){ctx.moveTo(-3,0);ctx.lineTo(3,0);ctx.moveTo(0,-3);ctx.lineTo(0,3);}
    }
    ctx.lineJoin='miter';ctx.strokeStyle='rgba(0,0,0,.8)';ctx.lineWidth=3;ctx.stroke();
    ctx.strokeStyle=ink;ctx.lineWidth=1;ctx.stroke();ctx.restore();
  }
}
export function nullMarkerAt(markers:NullMarker[],x:number,y:number):NullMarker|null {
  return [...markers].reverse().find(marker=>{
    if(Math.abs(marker.x-x)<=10&&Math.abs(marker.y-y)<=10)return true;
    return marker.corners?.some((a,i,points)=>{
      const b=points[(i+1)%points.length]!,dx=b.x-a.x,dy=b.y-a.y,length=dx*dx+dy*dy;
      const t=length?Math.max(0,Math.min(1,((x-a.x)*dx+(y-a.y)*dy)/length)):0;
      return Math.hypot(x-a.x-t*dx,y-a.y-t*dy)<=5;
    });
  }) ?? null;
}
