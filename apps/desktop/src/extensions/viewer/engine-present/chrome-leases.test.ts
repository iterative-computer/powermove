import { expect, it, vi } from 'vitest';
import { ChromeLeases } from './chrome-leases';
import type { FrameDescriptor } from './protocol';
const descriptor=(seq:number,index=0):FrameDescriptor=>({seq,poolIndex:index,poolGeneration:1,depthId:{seq,poolIndex:index*2,poolGeneration:2,encoding:'chromePlanesV1'}} as FrameDescriptor);

it('retains displayed depth after color GPU completion until replacement',()=>{
  const release=vi.fn(),leases=new ChromeLeases(release);
  leases.track(descriptor(1));leases.drawing(1);leases.showing(1);leases.colorDone({seq:1,poolIndex:0});
  expect(release).not.toHaveBeenCalled();
  leases.track(descriptor(2,1));leases.drawing(2);
  expect(release).not.toHaveBeenCalled();
  leases.showing(2);
  expect(release).toHaveBeenCalledExactlyOnceWith({seq:1,poolIndex:0});
});
it('waits for chrome fences and protects depth imports from retirement',async()=>{
  const release=vi.fn(),retire=vi.fn(()=>4),leases=new ChromeLeases(release);
  leases.track(descriptor(1));leases.drawing(1);leases.showing(1);
  const finish=leases.acquire(1)!;
  let resolve!:()=>void;const fence=new Promise<void>(done=>resolve=done);
  finish(fence);leases.colorDone({seq:1,poolIndex:0});
  expect(leases.retire(2,retire)).toBe(0);
  const next=descriptor(2,1);next.poolGeneration=3;next.depthId!.poolGeneration=4;
  leases.track(next);leases.drawing(2);leases.showing(2);
  expect(release).not.toHaveBeenCalled();expect(retire).not.toHaveBeenCalled();
  resolve();await fence;await Promise.resolve();
  expect(release).toHaveBeenCalledExactlyOnceWith({seq:1,poolIndex:0});
  expect(retire).toHaveBeenCalledOnce();
  leases.clear();leases.colorDone({seq:2,poolIndex:1});
  expect(retire).toHaveBeenCalledOnce();
});
it('releases dropped descriptors immediately and denies retired readers',()=>{
  const release=vi.fn(),leases=new ChromeLeases(release);
  leases.track(descriptor(3));leases.colorDone({seq:3,poolIndex:0});
  expect(release).toHaveBeenCalledExactlyOnceWith({seq:3,poolIndex:0});
  expect(leases.acquire(3)).toBeNull();
});
it('clears sessions only after active readers and color complete',async()=>{
  const release=vi.fn(),leases=new ChromeLeases(release);
  leases.track(descriptor(1));leases.drawing(1);leases.showing(1);
  const finish=leases.acquire(1)!;
  leases.clear();leases.colorDone({seq:1,poolIndex:0});
  expect(leases.acquire(1)).toBeNull();expect(release).not.toHaveBeenCalled();
  finish();finish();await Promise.resolve();
  expect(release).toHaveBeenCalledOnce();
});

it('does not retain stale descriptors after their pool was retired',()=>{
  const release=vi.fn(),retire=vi.fn(()=>2),leases=new ChromeLeases(release);
  expect(leases.retire(2,retire)).toBe(2);
  leases.track(descriptor(1));leases.drawing(1);leases.showing(1);
  expect(leases.acquire(1)).toBeNull();
  leases.clear();
  expect(retire).toHaveBeenCalledOnce();
});
