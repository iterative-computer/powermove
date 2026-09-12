export function textControlValues(PM:any,layer:any,time:number) {
  return ['animators','styles'].map(kind=>(layer.d[kind]||[]).map((item:any)=>({...item,p:Object.fromEntries(Object.entries(item.p).map(([k,p])=>[k,PM.evP(layer,p,time,`${kind==='animators'?'ta':'ts'}.${item.id}.${k}`)]))})));
}
export function animatorWeight(p:any,index:number,count:number):number {
  const position=(index+.5)/Math.max(1,count)*100-(Number(p.offset)||0),start=Math.min(p.start,p.end),end=Math.max(p.start,p.end);
  if(position<start||position>end)return 0;
  const feather=Math.max(0,Number(p.smoothness)||0)/100*(end-start)/2;
  if(!feather)return 1;const weight=Math.max(0,Math.min(1,(position-start)/feather,(end-position)/feather));return weight*weight*(3-2*weight);
}
export function animatedGlyphs(PM:any,layer:any,time:number,d:any,layout:any) {
  const [animators,styles]=textControlValues(PM,layer,time);
  return layout.characters.map((glyph:any)=>{let style={...d},x=glyph.x,y=glyph.y,rotation=0,scale=1,opacity=1;
    for(const range of styles)if(glyph.sourceStart>=range.start && glyph.sourceStart<range.end)style={...style,...range.p};
    for(const animator of animators){const p=animator.p,unit=p.unit||'characters',index=unit==='lines'?glyph.line:unit==='words'?glyph.word:glyph.index,count=layout[unit]?.length||layout.characters.length,w=animatorWeight(p,index,count);x+=(Number(p.x)||0)*w+(Number(p.tracking)||0)*glyph.index*w;y+=(Number(p.y)||0)*w;rotation+=(Number(p.rotation)||0)*w;scale*=1+((Number(p.scale)||0)/100-1)*w;opacity*=1+((Number(p.opacity)||0)/100-1)*w;}
    return {...glyph,style,x,y,rotation,scale,opacity:Math.max(0,Math.min(1,opacity))};
  });
}
