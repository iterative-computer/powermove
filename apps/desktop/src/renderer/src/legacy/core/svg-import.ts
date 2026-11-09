import { makeVectorPath, makeVertex, type VectorPath } from './vector-paths';

import { parseSvgPathData } from '../../../../shared/figma/paths';
export { parseSvgPathData } from '../../../../shared/figma/paths';
type Point = { x: number; y: number };
const vertex = (x:number,y:number): Vertex => ({x,y,inX:0,inY:0,outX:0,outY:0});
type Vertex = Point & { inX: number; inY: number; outX: number; outY: number };
type Matrix = [number, number, number, number, number, number];

export type ImportedSvgPath = {
  name: string;
  vertices: Vertex[];
  closed: boolean;
  fill: string;
  fillEnabled: boolean;
  fillOpacity: number;
  stroke: string;
  strokeWidth: number;
  strokeOpacity: number;
  fillGroup?: string;
  fillOpen?: boolean;
  fillRule?: 'nonzero' | 'evenodd';
  cap?: string;
  join?: 'miter' | 'round' | 'bevel';
  miterLimit?: number;
};

export type ImportedSvg = {
  width: number;
  height: number;
  paths: ImportedSvgPath[];
  warnings: string[];
};

const IDENTITY: Matrix = [1, 0, 0, 1, 0, 0];
const COMMAND = /^[a-z]$/i;
const NUMBER = /[-+]?(?:\d*\.\d+|\d+\.?)(?:e[-+]?\d+)?/ig;

function multiply(a: Matrix, b: Matrix): Matrix {
  return [
    a[0] * b[0] + a[2] * b[1], a[1] * b[0] + a[3] * b[1],
    a[0] * b[2] + a[2] * b[3], a[1] * b[2] + a[3] * b[3],
    a[0] * b[4] + a[2] * b[5] + a[4], a[1] * b[4] + a[3] * b[5] + a[5],
  ];
}

function apply(matrix: Matrix, point: Point): Point {
  return {
    x: matrix[0] * point.x + matrix[2] * point.y + matrix[4],
    y: matrix[1] * point.x + matrix[3] * point.y + matrix[5],
  };
}

function transformList(value: string | null): Matrix {
  let result: Matrix = IDENTITY;
  const pattern = /([a-z]+)\s*\(([^)]*)\)/ig;
  for (const match of value?.matchAll(pattern) || []) {
    const values = (match[2]!.match(NUMBER) || []).map(Number);
    let next: Matrix = IDENTITY;
    switch (match[1]!.toLowerCase()) {
      case 'matrix': if (values.length >= 6) next = values.slice(0, 6) as Matrix; break;
      case 'translate': next = [1, 0, 0, 1, values[0] || 0, values[1] || 0]; break;
      case 'scale': next = [values[0] ?? 1, 0, 0, values[1] ?? values[0] ?? 1, 0, 0]; break;
      case 'rotate': {
        const radians = (values[0] || 0) * Math.PI / 180;
        const rotation: Matrix = [Math.cos(radians), Math.sin(radians), -Math.sin(radians), Math.cos(radians), 0, 0];
        if (values.length >= 3) {
          const [cx, cy] = values.slice(1);
          next = multiply(multiply([1, 0, 0, 1, cx!, cy!], rotation), [1, 0, 0, 1, -cx!, -cy!]);
        } else next = rotation;
        break;
      }
      case 'skewx': next = [1, 0, Math.tan((values[0] || 0) * Math.PI / 180), 1, 0, 0]; break;
      case 'skewy': next = [1, Math.tan((values[0] || 0) * Math.PI / 180), 0, 1, 0, 0]; break;
    }
    result = multiply(result, next);
  }
  return result;
}


function styleMap(element: Element) {
  const result: Record<string, string> = {};
  for (const pair of (element.getAttribute('style') || '').split(';')) {
    const colon = pair.indexOf(':');
    if (colon > 0) result[pair.slice(0, colon).trim().toLowerCase()] = pair.slice(colon + 1).trim();
  }
  return result;
}

/** Resolve embedded author CSS without inserting the SVG into the page or
 * loading external sheets. Presentation attributes precede author rules. */
function svgCascade(svg:Element) {
  const rules:Array<{selector:string;declarations:Record<string,string>;specificity:number;order:number}>=[];
  for(const sheet of Array.from(svg.querySelectorAll('style'))){
    const source=(sheet.textContent || '').replace(/\/\*[\s\S]*?\*\//g,'');
    for(const match of source.matchAll(/([^{}]+)\{([^{}]*)\}/g)){
      const declarations=styleMap({getAttribute:()=>match[2]} as unknown as Element);
      for(const selector of match[1]!.split(',').map(s=>s.trim()).filter(s=>s&&!s.startsWith('@'))){
        const specificity=(selector.match(/#[\w-]+/g)?.length || 0)*10000+(selector.match(/\.[\w-]+|\[[^\]]+\]|:(?!:)[\w-]+/g)?.length || 0)*100+(selector.replace(/#[\w-]+|\.[\w-]+|\[[^\]]+\]|:[\w-]+(?:\([^)]*\))?/g,'').match(/[a-z_][\w-]*/ig)?.length || 0);
        rules.push({selector,declarations,specificity,order:rules.length});
      }
    }
  }
  return (element:Element)=>{
    const values:Record<string,{value:string;rank:number;order:number}>={};
    const add=(declarations:Record<string,string>,rank:number,order:number)=>{
      for(const [name,raw] of Object.entries(declarations)){
        const important=/\s*!important\s*$/i.test(raw),value=raw.replace(/\s*!important\s*$/i,'').trim(),weight=rank+(important?1e9:0),prior=values[name];
        if(!prior||weight>prior.rank||weight===prior.rank&&order>=prior.order)values[name]={value,rank:weight,order};
      }
    };
    for(const rule of rules){try{if(element.matches(rule.selector))add(rule.declarations,rule.specificity+1,rule.order);}catch{/* Unsupported selectors never execute code. */}}
    add(styleMap(element),1e7,rules.length);
    return Object.fromEntries(Object.entries(values).map(([key,record])=>[key,record.value]));
  };
}

function normalizedColor(input: string | null, fallback: string) {
  const value = String(input || fallback).trim().toLowerCase();
  const named: Record<string, string> = { black: '#000000', white: '#FFFFFF', red: '#FF0000', green: '#008000', blue: '#0000FF', transparent: '#00000000' };
  if (named[value]) return named[value];
  if (/^#[0-9a-f]{3}$/i.test(value)) return ('#' + [...value.slice(1)].map(c => c + c).join('')).toUpperCase();
  if (/^#[0-9a-f]{6}$/i.test(value)) return value.toUpperCase();
  if (/^#[0-9a-f]{8}$/i.test(value)) return value.toUpperCase();
  const rgb = /^rgba?\(([^)]+)\)$/i.exec(value);
  if (rgb) {
    const channels = rgb[1]!.split(',').slice(0, 3).map(part => part.trim().endsWith('%') ? Number.parseFloat(part) * 2.55 : Number(part));
    if (channels.every(Number.isFinite)) return '#' + channels.map(channel => Math.max(0, Math.min(255, Math.round(channel))).toString(16).padStart(2, '0')).join('').toUpperCase();
  }
  return fallback;
}

function numberAttr(element: Element, name: string, fallback = 0) {
  const value = Number.parseFloat(element.getAttribute(name) || '');
  return Number.isFinite(value) ? value : fallback;
}

function shapeData(element: Element): Array<{ vertices: Vertex[]; closed: boolean }> {
  const tag = element.localName.toLowerCase();
  if (tag === 'path') return parseSvgPathData(element.getAttribute('d') || '');
  if (tag === 'polygon' || tag === 'polyline') {
    const values = (element.getAttribute('points') || '').match(NUMBER)?.map(Number) || [];
    const vertices: Vertex[] = [];
    for (let i = 0; i + 1 < values.length; i += 2) vertices.push(vertex(values[i]!, values[i + 1]!));
    return vertices.length ? [{ vertices, closed: tag === 'polygon' }] : [];
  }
  if (tag === 'line') return [{ vertices: [vertex(numberAttr(element, 'x1'), numberAttr(element, 'y1')), vertex(numberAttr(element, 'x2'), numberAttr(element, 'y2'))], closed: false }];
  if (tag === 'rect') {
    const x = numberAttr(element, 'x'), y = numberAttr(element, 'y'), w = numberAttr(element, 'width'), h = numberAttr(element, 'height');
    const rx = Math.min(w / 2, Math.max(0, numberAttr(element, 'rx', numberAttr(element, 'ry'))));
    const ry = Math.min(h / 2, Math.max(0, numberAttr(element, 'ry', rx)));
    if (!rx && !ry) return [{ vertices: [vertex(x, y), vertex(x + w, y), vertex(x + w, y + h), vertex(x, y + h)], closed: true }];
    return parseSvgPathData(`M${x + rx} ${y} H${x + w - rx} A${rx} ${ry} 0 0 1 ${x + w} ${y + ry} V${y + h - ry} A${rx} ${ry} 0 0 1 ${x + w - rx} ${y + h} H${x + rx} A${rx} ${ry} 0 0 1 ${x} ${y + h - ry} V${y + ry} A${rx} ${ry} 0 0 1 ${x + rx} ${y} Z`);
  }
  if (tag === 'circle' || tag === 'ellipse') {
    const cx = numberAttr(element, 'cx'), cy = numberAttr(element, 'cy');
    const rx = tag === 'circle' ? numberAttr(element, 'r') : numberAttr(element, 'rx');
    const ry = tag === 'circle' ? rx : numberAttr(element, 'ry');
    return parseSvgPathData(`M${cx + rx} ${cy} A${rx} ${ry} 0 1 1 ${cx - rx} ${cy} A${rx} ${ry} 0 1 1 ${cx + rx} ${cy} Z`);
  }
  return [];
}

function viewportMatrix(svg: Element, width: number, height: number): Matrix {
  const parts = (svg.getAttribute('viewBox') || '').match(NUMBER)?.map(Number);
  if (!parts || parts.length < 4 || !(parts[2]! > 0 && parts[3]! > 0)) return [1, 0, 0, 1, -width / 2, -height / 2];
  const [minX, minY, viewWidth, viewHeight] = parts;
  const preserve = (svg.getAttribute('preserveAspectRatio') || 'xMidYMid meet').trim();
  if (/^none(?:\s|$)/i.test(preserve)) return [width / viewWidth!, 0, 0, height / viewHeight!, -width / 2 - minX! * width / viewWidth!, -height / 2 - minY! * height / viewHeight!];
  const scale = /\bslice\b/i.test(preserve) ? Math.max(width / viewWidth!, height / viewHeight!) : Math.min(width / viewWidth!, height / viewHeight!);
  const renderedWidth = viewWidth! * scale, renderedHeight = viewHeight! * scale;
  let offsetX = 0, offsetY = 0;
  if (/xmid/i.test(preserve)) offsetX = (width - renderedWidth) / 2; else if (/xmax/i.test(preserve)) offsetX = width - renderedWidth;
  if (/ymid/i.test(preserve)) offsetY = (height - renderedHeight) / 2; else if (/ymax/i.test(preserve)) offsetY = height - renderedHeight;
  return [scale, 0, 0, scale, -width / 2 + offsetX - minX! * scale, -height / 2 + offsetY - minY! * scale];
}

export function parseSvg(source: string,options:{legacy?:boolean}={}): ImportedSvg {
  const Parser = globalThis.DOMParser || (globalThis as any).window?.DOMParser;
  if (!Parser) throw new Error('SVG parsing is unavailable');
  const document = new Parser().parseFromString(source, 'image/svg+xml');
  if (document.querySelector('parsererror')) throw new Error('Could not parse this SVG file');
  const svg = document.documentElement;
  if (svg.localName.toLowerCase() !== 'svg') throw new Error('This file is not an SVG');
  const viewBox = (svg.getAttribute('viewBox') || '').match(NUMBER)?.map(Number);
  const rootLength = (name: string, fallback: number) => {
    const raw = svg.getAttribute(name) || '';
    return raw.includes('%') ? fallback : numberAttr(svg, name, fallback);
  };
  const width = rootLength('width', viewBox?.[2] || 0), height = rootLength('height', viewBox?.[3] || 0);
  if (!(width > 0 && height > 0)) throw new Error('SVG needs a positive width, height, or viewBox');
  const paths: ImportedSvgPath[] = [], warnings = new Set<string>(),cascade=svgCascade(svg);
  let fillGroup=0;
  const supported = new Set(['path', 'rect', 'circle', 'ellipse', 'polygon', 'polyline', 'line']);
  const containers = new Set(['svg', 'g', 'a', 'switch']);
  type Style = { fill: string; color:string; fillOpacity: number; stroke: string; strokeWidth: number; strokeOpacity: number; opacity: number; display: string; visibility: string; fillRule: string;cap:string;join:string;miterLimit:number };
  const walk = (element: Element, parentMatrix: Matrix, inherited: Style) => {
    const tag = element.localName.toLowerCase();
    if (['defs', 'clipPath', 'mask', 'pattern', 'linearGradient', 'radialGradient', 'metadata', 'title', 'desc','style','script'].map(v => v.toLowerCase()).includes(tag)) return;
    const inline = options.legacy?styleMap(element):cascade(element);
    const read = (name: string, fallback: string) => {const value=inline[name] ?? element.getAttribute(name);return !value||value==='inherit'||value==='unset'?fallback:value;};
    const style: Style = {
      fill: read('fill', inherited.fill), stroke: read('stroke', inherited.stroke),
      color:read('color',inherited.color),
      fillOpacity: Math.max(0, Math.min(1, Number.parseFloat(read('fill-opacity', String(inherited.fillOpacity))))),
      strokeWidth: Number.parseFloat(read('stroke-width', String(inherited.strokeWidth))) || 0,
      strokeOpacity: Math.max(0, Math.min(1, Number.parseFloat(read('stroke-opacity', String(inherited.strokeOpacity))))),
      opacity: inherited.opacity * Math.max(0, Math.min(1, Number.parseFloat(read('opacity', '1')))),
      display: read('display', inherited.display), visibility: read('visibility', inherited.visibility),
      fillRule: read('fill-rule', inherited.fillRule),
      cap:read('stroke-linecap',inherited.cap),join:read('stroke-linejoin',inherited.join),miterLimit:Number.parseFloat(read('stroke-miterlimit',String(inherited.miterLimit))),
    };
    if (style.display === 'none' || style.visibility === 'hidden') return;
    const matrix = multiply(parentMatrix, transformList(element.getAttribute('transform')));
    if (supported.has(tag)) {
      let subpaths: Array<{ vertices: Vertex[]; closed: boolean }> = [];
      try { subpaths = shapeData(element); } catch (error) { warnings.add(error instanceof Error ? error.message : `Skipped invalid ${tag}`); }
      const group=`svg-${fillGroup++}`;
      subpaths.forEach((subpath, index) => {
        const transformed = subpath.vertices.map(item => {
          const p = apply(matrix, item), incoming = apply(matrix, { x: item.x + item.inX, y: item.y + item.inY }), outgoing = apply(matrix, { x: item.x + item.outX, y: item.y + item.outY });
          return { x: p.x, y: p.y, inX: incoming.x - p.x, inY: incoming.y - p.y, outX: outgoing.x - p.x, outY: outgoing.y - p.y };
        });
        if (!transformed.length) return;
        if (/^url\(/i.test(style.fill) || /^url\(/i.test(style.stroke)) warnings.add('Gradient and pattern paints use a solid fallback');
        const strokeScale = Math.sqrt(Math.abs(matrix[0] * matrix[3] - matrix[1] * matrix[2]));
        paths.push({
          name: element.getAttribute('id') || `${tag[0]!.toUpperCase()}${tag.slice(1)} ${paths.length + 1}${subpaths.length > 1 ? `.${index + 1}` : ''}`,
          vertices: transformed, closed: subpath.closed,
          fill: normalizedColor(style.fill.toLowerCase()==='currentcolor'?style.color:style.fill, '#000000'), fillEnabled: (!options.legacy||subpath.closed)&&style.fill.toLowerCase() !== 'none',
          ...(!options.legacy?{fillGroup:group,fillOpen:true,fillRule:style.fillRule==='evenodd'?'evenodd' as const:'nonzero' as const}:{}),
          fillOpacity: style.fillOpacity * style.opacity * 100,
          stroke: normalizedColor(style.stroke.toLowerCase()==='currentcolor'?style.color:style.stroke, '#000000'), strokeWidth: style.stroke === 'none' ? 0 : Math.max(0, style.strokeWidth * strokeScale),
          strokeOpacity: style.strokeOpacity * style.opacity * 100,
          ...(!options.legacy?{cap:['round','square'].includes(style.cap)?style.cap.toUpperCase():'NONE',join:['round','bevel'].includes(style.join)?style.join as 'round'|'bevel':'miter' as const,miterLimit:Math.max(1,style.miterLimit || 4)}:{}),
        });
      });
    } else if (!containers.has(tag)) warnings.add(`Skipped unsupported <${tag}> content`);
    for (const child of Array.from(element.children)) walk(child, matrix, style);
  };
  walk(svg, viewportMatrix(svg, width, height), { fill: '#000000', color:'#000000',fillOpacity: 1, stroke: 'none', strokeWidth: 1, strokeOpacity: 1, opacity: 1, display: 'inline', visibility: 'visible', fillRule: 'nonzero',cap:'butt',join:'miter',miterLimit:4 });
  return { width, height, paths, warnings: [...warnings] };
}

export function materializeSvgPaths(PM: any, imported: ImportedSvg): VectorPath[] {
  return imported.paths.map(record => {
    const path = makeVectorPath(PM, record.name);
    Object.assign(path.p, {
      closed: PM.P(record.closed), fill: PM.P(record.fill), fillEnabled: PM.P(record.fillEnabled),
      fillOpacity: PM.P(record.fillOpacity), stroke: PM.P(record.stroke), strokeWidth: PM.P(record.strokeWidth), strokeOpacity: PM.P(record.strokeOpacity),
    });
    for(const key of ['fillGroup','fillOpen','fillRule','cap','join','miterLimit'] as const)if(record[key]!==undefined)path.p[key]=PM.P(record[key]);
    path.vertices = record.vertices.map(recordVertex => {
      const result = makeVertex(PM, recordVertex.x, recordVertex.y);
      for (const key of ['inX', 'inY', 'outX', 'outY'] as const) result.p[key].v = recordVertex[key];
      return result;
    });
    return path;
  });
}

/** Repair only untouched imports. Keep path/vertex identities, layer animation
 * and every user edit; callers put the returned content patch through history. */
export function svgImportRepair(layer:any,source:string,current:ImportedSvg):Record<string,unknown>|null {
  if(layer.lock||layer.d?.svgImportVersion>=2||!layer.d?.svgSourceAsset)return null;
  if(Object.keys(layer.locked_intent || {}).some(key=>key.startsWith('g.')||key.startsWith('c.paths')))return null;
  const factory={uid:()=>'',P:(v:unknown)=>({v,kf:[],expr:null})};
  const legacy=materializeSvgPaths(factory,parseSvg(source,{legacy:true})),saved=layer.d.paths;
  if(!Array.isArray(saved)||!saved.length||legacy.length!==current.paths.length)return null;
  const equalChannel=(a:any,b:any)=>a&&b&&!a.expr&&!a.kf?.length&&JSON.stringify(a.v)===JSON.stringify(b.v);
  const used=new Set<number>(),matches:number[]=[];
  for(const a of saved){
    const index=legacy.findIndex((b,i)=>!used.has(i)&&!a.parent&&a.vertices.length===b.vertices.length
      &&Object.entries(a.p).every(([key,ch])=>equalChannel(ch,b.p[key]))
      &&a.vertices.every((vertex:any,j:number)=>Object.entries(vertex.p).every(([key,ch])=>equalChannel(ch,b.vertices[j]!.p[key]))));
    if(index<0)return null;used.add(index);matches.push(index);
  }
  const native=materializeSvgPaths(factory,current);
  const updated=saved.map((path:any,i:number)=>({...path,p:{...path.p,...native[matches[i]!]!.p},vertices:path.vertices}));
  return {paths:updated,svgWarnings:current.warnings,svgImportVersion:2};
}
