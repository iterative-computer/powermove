type Mat3 = [
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number
];
type Mat4 = [
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number
];
/** Flat layers in a shared perspective space. Positive Z goes away from the viewer.
 * The default camera uses a 50 mm lens on a 36 mm film gate, centered on the comp. */
export const CHANNELS_3D = { perspective: 50, 'position.z': 0, 'anchor.z': 0, 'scale.z': 100, 'rotation.x': 0, 'rotation.y': 0, 'orientation.x': 0, 'orientation.y': 0, 'orientation.z': 0 };
const identity = (): Mat4 => [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
function multiply(a: Mat4, b: Mat4): Mat4 {
    return Array.from({ length: 16 }, (_, i) => { const r = i % 4, c = Math.floor(i / 4); let v = 0; for (let k = 0; k < 4; k++)
        v += a[k * 4 + r]! * b[c * 4 + k]!; return v; }) as Mat4;
}
function translation(x: number, y: number, z: number) { const m = identity(); m[12] = x; m[13] = y; m[14] = z; return m; }
function rotation(axis: number, degrees: number) { const m = identity(), a = (axis + 1) % 3, b = (axis + 2) % 3, c = Math.cos(degrees * Math.PI / 180), s = Math.sin(degrees * Math.PI / 180); m[a * 4 + a] = c; m[b * 4 + b] = c; m[a * 4 + b] = s; m[b * 4 + a] = -s; return m; }
function value(PM: any, L: any, key: string, T: number, fallback = 0): number { if (L.p?.[key] == null)
    return fallback; const v = Number(PM.ev(L, key, T)); return Number.isFinite(v) ? v : fallback; }
export function local3D(PM: any, L: any, T: number): Mat4 {
    const ev = (key: string, d = 0) => value(PM, L, key, T, d);
    if (!L.threeD) {
        const m = PM.localMatrix(L, T);
        return [m[0], m[1], 0, 0, m[2], m[3], 0, 0, 0, 0, 1, 0, m[4], m[5], 0, 1];
    }
    let m = translation(ev('position.x'), ev('position.y'), ev('position.z'));
    for (const [axis, key] of [[2, 'rotation'], [1, 'rotation.y'], [0, 'rotation.x'], [2, 'orientation.z'], [1, 'orientation.y'], [0, 'orientation.x']] as const)
        m = multiply(m, rotation(axis, ev(key)));
    const scale = identity();
    scale[0] = ev('scale.x', 100) / 100;
    scale[5] = ev('scale.y', 100) / 100;
    scale[10] = ev('scale.z', 100) / 100;
    scale[4] = Math.tan(ev('skew') * Math.PI / 180) * scale[5];
    return multiply(multiply(m, scale), translation(-ev('anchor.x'), -ev('anchor.y'), -ev('anchor.z')));
}
export function parent3D(PM: any, L: any, T: number, parent = L.parent ? PM.L(L.parent) : null): Mat4 {
    const chain: any[] = [], seen = new Set<any>(), groups = new Set<string>();
    let m = identity();
    for (let cur = L; cur && !seen.has(cur) && chain.length < 256; cur = cur === L ? parent : cur.parent ? PM.L(cur.parent) : null) {
        seen.add(cur);
        chain.push(cur);
    }
    for (const layer of chain)
        for (const group of (PM.groupAncestors?.(layer) || []).slice().reverse())
            if (!groups.has(group.id)) {
                groups.add(group.id);
                m = multiply(m, local3D(PM, group, T));
            }
    for (const layer of chain.reverse())
        if (layer !== L) m = multiply(m, local3D(PM, layer, T));
    return m;
}
export function world3D(PM: any, L: any, T: number, positioning = false): Mat4 {
    const own = positioning ? translation(0,0,L.threeD ? value(PM,L,'position.z',T) : 0) : local3D(PM,L,T);
    return multiply(parent3D(PM,L,T),own);
}

/** Groups supply one shared 3D coordinate space without changing member switches. */
export function is3DLayer(PM: any, layer: any): boolean {
    return !!layer.threeD || (PM.groupAncestors?.(layer) || []).some((group: any) => group.threeD);
}

/** The outermost enabled group owns the lens for its entire flat artwork. */
export function perspectiveAmount(PM: any, layer: any, time: number): number {
    const groups = (PM.groupAncestors?.(layer) || []).filter((group: any) => group.threeD);
    const owner = groups[groups.length - 1] || layer;
    return Math.max(0.001, value(PM, owner, 'perspective', time, 50));
}

/** Column-major homography, shared by GPU rendering, outlines and picking. */
export function planeMatrix(PM: any, L: any, T: number, positioning = false): Mat3 {
    if (!is3DLayer(PM,L)) {
        const m = PM.worldMatrix(L, T);
        return [m[0], m[1], 0, m[2], m[3], 0, m[4], m[5], 1];
    }
    const m = world3D(PM, L, T, positioning), comp = PM.curComp?.() || PM.proj, cx = comp.w / 2, cy = comp.h / 2, f = Math.max(1, comp.w) * perspectiveAmount(PM, L, T) / 36;
    return [m[0] + cx * m[2] / f, m[1] + cy * m[2] / f, m[2] / f, m[4] + cx * m[6] / f, m[5] + cy * m[6] / f, m[6] / f, m[12] + cx * m[14] / f, m[13] + cy * m[14] / f, 1 + m[14] / f];
}
export function projectPoint(m: Readonly<Mat3>, p: {
    x: number;
    y: number;
}): {
    x: number;
    y: number;
} { const w = m[2] * p.x + m[5] * p.y + m[8]; return { x: (m[0] * p.x + m[3] * p.y + m[6]) / w, y: (m[1] * p.x + m[4] * p.y + m[7]) / w }; }
export function inversePlane(m: Readonly<Mat3>): Mat3 | null {
    const [a, d, g, b, e, h, c, f, i] = m, A = e * i - f * h, B = f * g - d * i, C = d * h - e * g, det = a * A + b * B + c * C;
    if (!Number.isFinite(det) || Math.abs(det) < 1e-10)
        return null;
    return [A, B, C, c * h - b * i, a * i - c * g, b * g - a * h, b * f - c * e, c * d - a * f, a * e - b * d].map(v => v / det) as Mat3;
}
export function planeContains(PM: any, L: any, T: number, x: number, y: number, b: any): boolean {
    const m = planeMatrix(PM, L, T), inv = inversePlane(m);
    if (!inv)
        return false;
    const p = projectPoint(inv, { x, y });
    return m[2] * p.x + m[5] * p.y + m[8] > 0.0001 && p.x >= b.x0 && p.x <= b.x1 && p.y >= b.y0 && p.y <= b.y1;
}
/** Coplanar artwork is one compositing surface: preserve its timeline order.
 * Sorting each member's anchor independently lets a tilted card background hide
 * its own text. Different surfaces still sort by depth within each 2D barrier. */
export function depthOrderedLayers(PM: any, layers: any[], T: number): any[] {
    const result = layers.slice();
    let start = 0;
    while (start < result.length) {
        if (!is3DLayer(PM, result[start])) { start++; continue; }
        let end = start + 1;
        while (end < result.length && is3DLayer(PM, result[end])) end++;
        const surfaces: Array<{ plane: number[] | null; layers: any[]; depth: number }> = [];
        for (const layer of result.slice(start, end)) {
            const m = world3D(PM, layer, T);
            const normal = [m[1]*m[6]-m[2]*m[5], m[2]*m[4]-m[0]*m[6], m[0]*m[5]-m[1]*m[4]];
            const length = Math.hypot(...normal);
            let plane: number[] | null = null;
            if (length > 1e-12) {
                // A reflected layer still belongs to the same two-sided plane.
                const sign = (normal.find(v => Math.abs(v) > length*1e-8) ?? 1) < 0 ? -1 : 1;
                const n = normal.map(v => v/length*sign);
                plane = [...n, n[0]!*m[12]+n[1]!*m[13]+n[2]!*m[14]];
            }
            const existing = plane && surfaces.find(surface => surface.plane
                && plane!.slice(0,3).every((v,i) => Math.abs(v-surface.plane![i]!) < 1e-7)
                && Math.abs(plane![3]!-surface.plane[3]!) < 1e-5);
            if (existing) { existing.layers.push(layer); continue; }
            const depth = m[2]*value(PM,layer,'anchor.x',T)+m[6]*value(PM,layer,'anchor.y',T)
                +m[10]*(layer.threeD ? value(PM,layer,'anchor.z',T) : 0)+m[14];
            surfaces.push({plane, layers:[layer], depth});
        }
        result.splice(start,end-start,...surfaces.sort((a,b)=>a.depth-b.depth).flatMap(surface=>surface.layers));
        start=end;
    }
    return result;
}

/** Affine 3D inverse used when rebasing editable group/parent transforms. */
export function inverse3D(m: readonly number[]): Mat4 | null {
    const linear = inversePlane([m[0]!,m[1]!,m[2]!,m[4]!,m[5]!,m[6]!,m[8]!,m[9]!,m[10]!]);
    if (!linear) return null;
    const [a,b,c,d,e,f,g,h,i] = linear, x=m[12]!,y=m[13]!,z=m[14]!;
    return [a,b,c,0,d,e,f,0,g,h,i,0,-a*x-d*y-g*z,-b*x-e*y-h*z,-c*x-f*y-i*z,1];
}

/** Decompose into real position, rotation, scale and XY skew channels. Refuse
 * non-representable XZ/YZ shear instead of silently changing the visible pose. */
export function poseValues3D(PM:any,L:any,world:readonly number[],T:number,parent=parent3D(PM,L,T)):Record<string,number> {
    const inv=inverse3D(parent);
    if(!inv)throw new Error('Increase the parent’s zero scale before changing its 3D hierarchy');
    const m=multiply(inv,world as Mat4);
    const sx=Math.hypot(m[0],m[1],m[2]);
    if(sx<1e-9)throw new Error('Increase zero scale before changing the 3D hierarchy');
    const x=[m[0]/sx,m[1]/sx,m[2]/sx];
    const xy=x[0]!*m[4]+x[1]!*m[5]+x[2]!*m[6];
    const y=[m[4]-xy*x[0]!,m[5]-xy*x[1]!,m[6]-xy*x[2]!];
    const sy=Math.hypot(...y);
    if(sy<1e-9)throw new Error('Increase zero scale before changing the 3D hierarchy');
    for(let i=0;i<3;i++)y[i]=y[i]!/sy;
    const z=[x[1]!*y[2]!-x[2]!*y[1]!,x[2]!*y[0]!-x[0]!*y[2]!,x[0]!*y[1]!-x[1]!*y[0]!];
    const sz=z[0]!*m[8]+z[1]!*m[9]+z[2]!*m[10];
    const shearX=x[0]!*m[8]+x[1]!*m[9]+x[2]!*m[10],shearY=y[0]!*m[8]+y[1]!*m[9]+y[2]!*m[10];
    if(Math.max(Math.abs(shearX),Math.abs(shearY))>1e-6*Math.max(1,Math.abs(sz)))throw new Error('This 3D hierarchy has depth shear. Use uniform group scale before ungrouping or reparenting');
    const ry=Math.asin(Math.max(-1,Math.min(1,-x[2]!))),regular=Math.abs(Math.cos(ry))>1e-7;
    const rx=regular?Math.atan2(y[2]!,z[2]!):0,rz=regular?Math.atan2(x[1]!,x[0]!):Math.atan2(-y[0]!,y[1]!);
    const ax=value(PM,L,'anchor.x',T),ay=value(PM,L,'anchor.y',T),az=L.threeD?value(PM,L,'anchor.z',T):0;
    return {
        'position.x':m[12]+m[0]*ax+m[4]*ay+m[8]*az,
        'position.y':m[13]+m[1]*ax+m[5]*ay+m[9]*az,
        'position.z':m[14]+m[2]*ax+m[6]*ay+m[10]*az,
        'scale.x':sx*100,'scale.y':sy*100,'scale.z':sz*100,
        rotation:rz*180/Math.PI,'rotation.x':rx*180/Math.PI,'rotation.y':ry*180/Math.PI,
        'orientation.x':0,'orientation.y':0,'orientation.z':0,skew:Math.atan(xy/sy)*180/Math.PI,
    };
}
