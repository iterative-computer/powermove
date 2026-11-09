#!/usr/bin/env python3
"""Exercise the real process and check rendered pixels. Writes evidence only outside Git."""
import argparse, json, os, pathlib, statistics, struct, subprocess, time

def main():
    p=argparse.ArgumentParser();p.add_argument('--host',default=str(pathlib.Path(__file__).resolve().parent.parent/'resources/studio/pm-studio-host'));p.add_argument('--out',default='/tmp/pm-shots/studio3d/laneB');p.add_argument('--size',default='1920x1080');p.add_argument('--frames',type=int,default=30);p.add_argument('--scenes',default='hero,edge,macro,back,chrome,materials,caustics,wet');args=p.parse_args()
    out=pathlib.Path(args.out);out.mkdir(parents=True,exist_ok=True)
    host=subprocess.Popen([args.host],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=open(out/'host-stderr.log','w'))
    seq=0
    def read(n):
        b=bytearray()
        while len(b)<n:
            data=host.stdout.read(n-len(b))
            if not data:raise RuntimeError(f'host exited {host.poll()} while reading')
            b.extend(data)
        return bytes(b)
    def request(op,binary=b'',**kw):
        nonlocal seq
        seq+=1;data=json.dumps(dict(id=seq,op=op,**kw)).encode();start=time.perf_counter()
        host.stdin.write(struct.pack('<4sII',b'PMS1',len(data),len(binary))+data+binary);host.stdin.flush()
        magic,j,b=struct.unpack('<4sII',read(12));assert magic==b'PMS1';response=json.loads(read(j));pixels=read(b)
        if not response['ok']:raise RuntimeError(response['error'])
        return response['result'],pixels,(time.perf_counter()-start)*1000
    def png(name,info,pixels):
        import zlib
        w,h=info['width'],info['height']
        def chunk(typ,data):return struct.pack('>I',len(data))+typ+data+struct.pack('>I',zlib.crc32(typ+data))
        raw=b''.join(b'\0'+pixels[y*w*4:(y+1)*w*4]for y in range(h))
        (out/name).write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',w,h,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b''))
    print(json.dumps({'hello':request('hello')[0]}),flush=True)
    w,h=map(int,args.size.split('x'));rows=[]
    try:
        for scene in args.scenes.split(','):
            request('preset',name=scene)
            bounds=request('bounds')[0];assert bounds['min'][0]<bounds['max'][0]
            info,pixels,cold=request('render',width=w,height=h,tier='live');assert len(pixels)==info['width']*info['height']*4
            assert max(pixels[::4])-min(pixels[::4])>16,'frame has no useful color variation'
            png(f'{scene}-native-live.png',info,pixels)
            times=[];trace=[];post=[]
            for frame in range(args.frames):
                # A camera patch is an actual input, and never contains a mesh.
                request('patch',time=frame/60)
                meta,_,ms=request('render',width=w,height=h,tier='live',time=frame/60)
                times.append(ms);trace.append(meta['timings']['traceMs']);post.append(meta['timings']['postMs'])
            info,pixels,refine=request('render',width=w,height=h,tier='refine',samples=16,reset=True)
            png(f'{scene}-native-refine.png',info,pixels)
            rss=int(subprocess.check_output(['ps','-o','rss=','-p',str(host.pid)]).strip())/1024
            row=dict(scene=scene,liveHostMs=statistics.median(trace)+statistics.median(post),liveRoundTripMs=statistics.median(times),liveP95Ms=sorted(times)[int((len(times)-1)*.95)],traceMs=statistics.median(trace),postMs=statistics.median(post),refineSeconds=refine/1000,refineHostSeconds=info['timings']['hostMs']/1000,coldLiveMs=cold,rssMiB=rss,hardwareRt=info['hardwareRt'],denoiser=info['denoiser'])
            rows.append(row);print(json.dumps(row),flush=True);(out/'native-benchmark.json').write_text(json.dumps(rows,indent=2))
        # The process stays usable after a rejected request.
        try:request('render',width=0,height=h)
        except RuntimeError:pass
        assert request('hello')[0]['protocol']==1
        request('shutdown')
    finally:
        host.stdin.close()
        try:host.wait(timeout=5)
        except subprocess.TimeoutExpired:host.kill();host.wait()
    print('PASS native host smoke',flush=True)
if __name__=='__main__':main()
