/** Structural gate shared by captures and future native/compat replay readers. */
export function validateTrace(jsonl: string) {
  const rows=jsonl.trimEnd().split('\n').map(line=>JSON.parse(line));
  const header=rows[0], end=rows.at(-1);
  if(header?.format!=='powermove.behavior'||header.version!==1||header.type!=='header') throw new Error('Unsupported trace header');
  if(end?.type!=='end'||end.valid!==true) throw new Error('Incomplete or invalid trace');
  const stack:number[]=[];let calls=0;
  for(let i=1;i<rows.length;i++) {
    const row=rows[i];if(row.seq!==i) throw new Error(`Broken sequence at ${i}`);
    if(row.type==='call') {
      if(row.call!==++calls||row.parent!==(stack.at(-1)??null)) throw new Error(`Broken call at ${i}`);
      stack.push(row.call);
    } else if(row.type==='return'||row.type==='throw') {
      if(stack.pop()!==row.call) throw new Error(`Broken completion at ${i}`);
    } else if(row.parent!==undefined&&row.parent!==(stack.at(-1)??null)) throw new Error(`Broken event parent at ${i}`);
  }
  if(stack.length||end.calls!==calls) throw new Error('Unfinished calls');
  return {rows:rows.length,calls};
}
