import { animatedGlyphs, textAnimationKey, textAnimationState } from '../../legacy/core/text-animation';
import * as THREE from 'three';
import { parseSvgPathData } from '../../legacy/core/svg-import';
import type { Font } from 'fontkit';
import { textSourceLines, glyphLayout } from '../../legacy/core/text-layout';
import { pathValues, groupMatrix, tracePath } from '../../legacy/core/vector-paths';
import { roundedRectPath } from '../../legacy/gl/corner-geometry';
import { resolveContent } from '../../legacy/core/content-properties';
import { resolveContours, strokeContours, subtractOutlines, type Outline, type Point } from './extrusion-geometry';
import { requestExtrusionFont } from './extrusion-fonts';
import { readExtrusion } from './extrusion-data';

export const geometryDivisions=(quality:string)=>quality==='draft'?4:quality==='high'?20:10;
function commandContours(commands:any[],divisions:number):Point[][] {
  const paths:THREE.Path[]=[];let current=new THREE.Path();
  for(const c of commands){
    if(c.type==='M'){current=new THREE.Path();paths.push(current);current.moveTo(c.x,c.y);}
    else if(c.type==='L')current.lineTo(c.x,c.y);
    else if(c.type==='Q')current.quadraticCurveTo(c.x1,c.y1,c.x,c.y);
    else if(c.type==='C')current.bezierCurveTo(c.x1,c.y1,c.x2,c.y2,c.x,c.y);
    else if(c.type==='Z')current.closePath();
  }
  return paths.map(p=>p.getPoints(divisions).map(p=>[p.x,p.y] as Point));
}
export function fontOutlines(font:Font,d:any,divisions=10):Outline[]{
  const size=Math.max(1,Number(d.size)||16),variation=Object.fromEntries(Object.entries<any>(d).filter(([k])=>/^fontAxis\..{4}$/.test(k)).map(([k,v])=>[k.slice(9),v]));
  if(Object.keys(font.variationAxes).length){
    const axes=Object.fromEntries(Object.entries(font.variationAxes).map(([key,axis])=>[key,axis!.default]));
    font=font.getVariation({...axes,wght:Number(d.weight)||500,...(d.italic?{ital:1}:{}),...variation});
  }
  const scale=size/font.unitsPerEm,tracking=Number(d.tracking)||0;
  const runs=new Map<string,ReturnType<Font['layout']>>();
  const layout=(text:string)=>{let run=runs.get(text);if(!run){run=font.layout(text);runs.set(text,run);}return run;};
  const measure={measureText:(text:string)=>{const run=layout(text);return {width:run.positions.reduce((sum,p)=>sum+p.xAdvance*scale,0)+run.glyphs.length*tracking};}};
  const lh=size*(Number(d.leading)||1.15),lines=textSourceLines(d,measure,lh),out:Outline[]=[];
  for(let i=0;i<lines.length;i++){
    const text=lines[i]!.text,width=measure.measureText(text).width;let x=d.align==='center'?-width/2:d.align==='right'?-width:0;
    const run=layout(text);
    for(let j=0;j<run.glyphs.length;j++){
      const glyph=run.glyphs[j]!,p=run.positions[j]!,commands:any[]=[];
      for(const command of glyph.path.commands){
        const args=command.args.map((value:number,index:number)=>index%2?i*lh+size*.82-(value+p.yOffset)*scale:x+(value+p.xOffset)*scale);
        const type=({moveTo:'M',lineTo:'L',quadraticCurveTo:'Q',bezierCurveTo:'C',closePath:'Z'} as Record<string,string>)[command.command];
        if(type==='Q')commands.push({type,x1:args[0],y1:args[1],x:args[2],y:args[3]});
        else if(type==='C')commands.push({type,x1:args[0],y1:args[1],x2:args[2],y2:args[3],x:args[4],y:args[5]});
        else commands.push({type,x:args[0],y:args[1]});
      }
      out.push(...resolveContours(commandContours(commands,divisions)));
      x+=p.xAdvance*scale+tracking;
    }
  }
  return out;
}
function pathContours(values:any,divisions:number):Point[][]{
  const paths:THREE.Path[]=[];let current=new THREE.Path();
  // Reuse the renderer's exact trim and cubic construction, using a vector
  // recorder in place of Canvas. Trimmed/open paths become stroke outlines.
  const recorder={beginPath(){},moveTo(x:number,y:number){current=new THREE.Path();paths.push(current);current.moveTo(x,y);},lineTo(x:number,y:number){current.lineTo(x,y);},bezierCurveTo(...p:number[]){current.bezierCurveTo(...p as [number,number,number,number,number,number]);},closePath(){current.closePath();}};
  tracePath(recorder as any,values);
  return paths.map(p=>p.getPoints(divisions).map(p=>[p.x,p.y] as Point));
}
export function shapeOutlines(d:any,divisions=10,records?:any[]):Outline[]{
  if(records){
    const out:Outline[]=[],fills=new Map<string,{contours:Point[][];strokes:Outline[];rule:'nonzero'|'evenodd';color:string;opacity:number}>();
    for(const {v,matrix} of records)for(let copy=0;copy<Math.min(256,Math.max(1,Math.round(v.copies)));copy++){
      const angle=copy*v.repeatRotation*Math.PI/180,c=Math.cos(angle),s=Math.sin(angle);
      const transform=([x,y]:Point):Point=>{const xx=c*x-s*y+copy*v.repeatX,yy=s*x+c*y+copy*v.repeatY;return [matrix[0]*xx+matrix[2]*yy+matrix[4],matrix[1]*xx+matrix[3]*yy+matrix[5]];};
      const contours=pathContours(v,divisions),full=v.trimEnd-v.trimStart>=99.999;
      const strokes=v.strokeWidth>0?contours.flatMap(contour=>strokeContours(contour,v.strokeWidth,!!v.closed&&full)):[];
      if(full && (v.closed||v.fillOpen) && v.fillEnabled){
        const key=JSON.stringify([v.fillGroup??'legacy',copy,v.fill,v.fillOpacity,v.fillRule]);
        let group=fills.get(key);if(!group){group={contours:[],strokes:[],rule:v.fillRule==='evenodd'?'evenodd':'nonzero',color:v.fill,opacity:v.fillOpacity/100};fills.set(key,group);}
        group.contours.push(...contours.map(r=>r.map(transform)));
        group.strokes.push(...strokes.map(o=>({...o,outer:o.outer.map(transform),holes:o.holes.map(h=>h.map(transform))})));
      }
      out.push(...strokes.map(o=>({...o,outer:o.outer.map(transform),holes:o.holes.map(h=>h.map(transform)),color:v.stroke,opacity:v.strokeOpacity/100})));
    }
    // Matching vector fills form compound paths, including nested counters.
    for(const group of fills.values())out.unshift(...subtractOutlines(resolveContours(group.contours,group.rule),group.strokes).map(o=>({...o,color:group.color,opacity:group.opacity})));
    return out;
  }
  const w=Math.max(.001,Number(d.w)||100),h=Math.max(.001,Number(d.h)||100);let contours:Point[][]=[];
  if(d.shape==='ellipse'){
    const points:Point[]=[];for(let i=0;i<divisions*8;i++){const a=i/(divisions*8)*Math.PI*2;points.push([Math.cos(a)*w/2,Math.sin(a)*h/2]);}contours=[points];
  }else if(d.shape==='polygon' || d.shape==='star'){
    const n=Math.min(256,Math.max(3,Math.round(d.points || 5))),count=d.shape==='star'?n*2:n,r=Math.min(w,h)/2;
    contours=[Array.from({length:count},(_,i)=>{const a=-Math.PI/2+i/count*Math.PI*2,rad=d.shape==='star' && i%2?r*.46:r;return [Math.cos(a)*rad,Math.sin(a)*rad] as Point;})];
  }else if(d.shape==='line')return strokeContours([[-w/2,0],[w/2,0]],Math.max(1,d.stroke || 6),false);
  else{
    contours=parseSvgPathData(roundedRectPath({...d,w,h})).flatMap(path=>pathContours({...path,trimStart:0,trimEnd:100},divisions).map(ring=>ring.map(([x,y])=>[x-w/2,y-h/2] as Point)));
  }
  const strokes=d.stroke>0?contours.flatMap(contour=>strokeContours(contour,d.stroke,true)):[];
  return [...subtractOutlines(resolveContours(contours),strokes),...strokes.map(o=>({...o,color:d.strokeColor || '#FFFFFF'}))];
}
function animatedTextOutlines(PM:any,layer:any,time:number,d:any,divisions:number,layout:any,state:any):Outline[]|null {
  const glyphs=animatedGlyphs(PM,layer,time,d,layout,state),out:Outline[]=[];
  for(const g of glyphs){
    if(g.opacity<=0)continue;
    const style={...g.style,text:g.text,align:'left',paragraph:false,boxWidth:0,boxHeight:0},entry=requestExtrusionFont(PM,style);
    if(entry.error)throw new Error(entry.error);if(!entry.font)return null;
    const baseline=(Number(style.size)||16)*.82,m=g.matrix;
    const transform=([x,y]:Point):Point=>{y-=baseline;return [g.x+m[0]*x+m[2]*y+m[4],g.y+(Number(d.size)||16)*.82+m[1]*x+m[3]*y+m[5]];};
    for(const o of fontOutlines(entry.font,style,divisions))out.push({...o,outer:o.outer.map(transform),holes:o.holes.map(h=>h.map(transform)),opacity:g.opacity,
      ...(g.color || g.style.color!==d.color?{color:g.color || g.style.color}:{})});
  }
  return out;
}
const layouts=new WeakMap<object,Map<string,any>>();
export interface ExtrusionOutlineSample {key:string;outlines:Outline[];color:string;options:NonNullable<ReturnType<typeof readExtrusion>>['values']}
const cache=new WeakMap<object,Map<string,ExtrusionOutlineSample>>();
export function layerExtrusionOutlines(PM:any,layer:any,time:number):ExtrusionOutlineSample|null{
  const options=readExtrusion(PM,layer,time)!.values,d=resolveContent(PM,layer,time),divisions=geometryDivisions(options.quality);
  const records=layer.type==='shape' && d.paths?.length?d.paths.map((path:any)=>({v:pathValues(PM,layer,path,time),matrix:groupMatrix(PM,layer,path,time)})):undefined;
  // Exclude material/transform fields so those edits never rebuild a title.
  const fields=layer.type==='text'?['text','font','weight','italic','size','tracking','leading','align','paragraph','boxWidth','boxHeight',...Object.keys(d).filter(k=>k.startsWith('fontAxis.'))]
    :['shape','w','h','radius','independentCorners','radiusTL','radiusTR','radiusBR','radiusBL','smoothing','points','stroke','strokeColor'];
  let animation:any,layout:any;
  const typography=JSON.stringify(fields.map(k=>d[k]));
  if(layer.type==='text' && (d.animators?.length || d.styles?.length)){
    let pool=layouts.get(PM);if(!pool){pool=new Map();layouts.set(PM,pool);}
    layout=pool.get(typography);
    if(!layout){
      const entry=requestExtrusionFont(PM,d);if(entry.error)throw new Error(entry.error);if(!entry.font)return null;
      const font=entry.font,size=Number(d.size)||16;
      layout=PM.textLayout?.(layer,time) || glyphLayout(d,{measureText:(text:string)=>({width:font.layout(text).positions.reduce((sum,p)=>sum+p.xAdvance,0)*size/font.unitsPerEm+text.length*(Number(d.tracking)||0)})});
      pool.set(typography,layout);if(pool.size>32)pool.delete(pool.keys().next().value!);
    }
    animation=textAnimationState(PM,layer,time,layout);
  }
  const key=JSON.stringify([layer.type,typography,records,options,animation?[textAnimationKey(animation),d.color]:null]);
  let pool=cache.get(PM);if(!pool){pool=new Map();cache.set(PM,pool);}
  let sample=pool.get(key);
  if(!sample){
    let font:Font|undefined;
    if(layer.type==='text'){const entry=requestExtrusionFont(PM,d);if(entry.error)throw new Error(entry.error);font=entry.font;if(!font)return null;}
    const outlines=layer.type==='text'?(animation?animatedTextOutlines(PM,layer,time,d,divisions,layout,animation):fontOutlines(font!,d,divisions)):shapeOutlines(d,divisions,records);
    if(!outlines)return null;
    sample={key,outlines,color:'#FFFFFF',options};pool.set(key,sample);
    if(pool.size>32)pool.delete(pool.keys().next().value!);
  }
  return {...sample,color:d.color || '#E8E2CF'};
}
