/** CSS artwork paint split for renderers whose color type has no alpha. */
export function splitPaintColor(input:string):{color:string;opacity:number} {
  const value=String(input).trim();
  if(/^(transparent|none)$/i.test(value))return {color:'#000000',opacity:0};
  const hex=/^#([\da-f]{4}|[\da-f]{8})$/i.exec(value);
  if(hex){
    const full=hex[1]!.length===4?[...hex[1]!].map(c=>c+c).join(''):hex[1]!;
    return {color:'#'+full.slice(0,6),opacity:parseInt(full.slice(6),16)/255};
  }
  const rgb=/^rgba?\(([^)]+)\)$/i.exec(value);
  if(rgb){
    const parts=rgb[1]!.trim().split(/[\s,/]+/),alpha=parts[3];
    if(alpha!==undefined){
      const opacity=Number.parseFloat(alpha)/(alpha.endsWith('%')?100:1);
      return {color:`rgb(${parts.slice(0,3).join(',')})`,opacity:Number.isFinite(opacity)?Math.max(0,Math.min(1,opacity)):1};
    }
  }
  return {color:value,opacity:1};
}
