"""Generated display-matrix witness, with an exact FFmpeg autorotate reference."""
import json
from pathlib import Path
import shutil
import subprocess

class UnsupportedFFmpeg(RuntimeError):
    pass

def generate(root, turns):
    if not shutil.which('ffmpeg') or not shutil.which('ffprobe'):
        raise UnsupportedFFmpeg('ffmpeg/ffprobe unavailable')
    plain, rotated = root/'plain.mov', root/('rotation-%d.mov' % turns)
    def run(args):
        result = subprocess.run(args, capture_output=True)
        if result.returncode:
            raise UnsupportedFFmpeg('FFmpeg cannot generate/probe display-matrix fixture: ' + result.stderr.decode(errors='replace'))
        return result.stdout
    if not plain.exists():
        run(['ffmpeg','-v','error','-f','lavfi','-i','testsrc2=size=64x48:rate=30:duration=1',
             '-c:v','mpeg4','-q:v','2','-pix_fmt','yuv420p',str(plain)])
    # New FFmpeg versions use an input display-matrix option; older builds
    # accept output rotate metadata. Probe the result instead of assuming either.
    attempts = [
        ['ffmpeg','-v','error','-display_rotation',str(turns*90),'-i',str(plain),'-c','copy','-y',str(rotated)],
        ['ffmpeg','-v','error','-i',str(plain),'-c','copy','-metadata:s:v:0','rotate=%d' % (turns*90),'-y',str(rotated)],
    ]
    for command in attempts:
        try:
            run(command)
            probe = json.loads(run(['ffprobe','-v','error','-show_entries','stream_side_data=rotation','-of','json',str(rotated)]))
            rotations = [d['rotation'] for s in probe.get('streams',[]) for d in s.get('side_data_list',[]) if 'rotation' in d]
            if rotations and rotations[0] % 360 == turns*90:
                break
        except UnsupportedFFmpeg:
            continue
    else:
        raise UnsupportedFFmpeg('FFmpeg did not preserve the requested display matrix with either supported syntax')
    def gray(path, auto):
        return run(['ffmpeg','-v','error'] + ([] if auto else ['-noautorotate']) +
                   ['-i',str(path),'-frames:v','1','-f','rawvideo','-pix_fmt','gray','-'])
    base, reference = gray(plain, False), gray(rotated, True)
    w,h = 64,48
    ow,oh = (h,w) if turns % 2 else (w,h)
    def source(x,y):
        return [(x,y),(w-1-y,x),(w-1-x,h-1-y),(y,h-1-x)][turns]
    mapped = bytes(base[v*w+u] for y in range(oh) for x in range(ow) for u,v in [source(x,y)])
    if mapped != reference:
        raise AssertionError('display-matrix permutation differs from FFmpeg autorotate')
    return {'plain':str(plain),'rotated':str(rotated),'turns':turns}
