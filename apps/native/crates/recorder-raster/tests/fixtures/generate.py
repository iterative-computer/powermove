#!/usr/bin/env python3
"""Generate synthetic Canvas PNGs using an isolated headless Chromium profile.
No installed fonts, user projects, cookies, external URLs or reference screenshots.
Run from any directory. Chromium is a build-time oracle, never an app dependency.
"""
import base64
import hashlib
import asyncio
import os
import signal
import time
import json
from pathlib import Path
import subprocess
import tempfile

SCENES = r"""
const names=['rects','curves','arcs','roundrect','strokes','clip-transform','linear','radial','conic','patterns','images','composites','text'];
const modes=['source-over','source-in','source-out','source-atop','destination-over','destination-in','destination-out','destination-atop','lighter','copy','xor','multiply','screen','overlay','darken','lighten','color-dodge','color-burn','hard-light','soft-light','difference','exclusion','hue','saturation','color','luminosity'];
function image(){const cv=document.createElement('canvas');cv.width=cv.height=2;cv.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray([255,0,0,255,0,255,0,255,0,0,255,255,255,255,255,255]),2,2),0,0);return cv;}
const out={};
for(const name of names){
 const cv=document.createElement('canvas'); cv.width=name==='composites'?26:32;cv.height=name==='composites'?3:24;
 const c=cv.getContext('2d');
 switch(name){
 case 'text':
  c.font="10px 'Recorder Test'";c.fillText('AB',2,10);c.fillText('AA',2,22,8);
  c.textAlign='center';c.textBaseline='top';c.fillText('A',24,2);
  c.textAlign='left';c.textBaseline='alphabetic';c.strokeText('B',20,22);break;
 case 'rects':
  c.fillStyle='#e03010';c.fillRect(1,1,12,10);c.fillStyle='#2080e0';c.globalAlpha=.5;c.fillRect(7,7,16,12);c.clearRect(9,9,3,3);c.globalAlpha=1;
  c.fillStyle='#10b060';c.fillRect(25.25,2.25,4.5,4.5);c.strokeStyle='black';c.lineWidth=2;c.strokeRect(2,15,4,6);break;
 case 'curves':
  c.fillStyle='#e03010';c.moveTo(1,2);c.lineTo(15,3);c.lineTo(3,11);c.closePath();c.fill();
  c.beginPath();c.moveTo(17,2);c.quadraticCurveTo(30,1,30,11);c.bezierCurveTo(20,16,25,8,17,2);c.fill();
  c.beginPath();c.rect(2,15,12,7);c.rect(5,17,6,3);c.fill('evenodd');break;
 case 'arcs':
  c.arc(8,8,5,0,Math.PI*2);c.fill();c.beginPath();c.ellipse(23,7,6,3,.3,0,Math.PI*2);c.fill();
  c.beginPath();c.moveTo(2,21);c.arcTo(2,14,13,14,3);c.lineTo(13,21);c.closePath();c.fill();
  c.beginPath();c.arc(24,19,4,Math.PI,0,true);c.stroke();break;
 case 'roundrect':
  c.roundRect(1,1,14,21,4);c.fill();c.beginPath();c.roundRect(30,22,-13,-20,[{x:2,y:4},{x:5,y:3}]);c.fill();break;
 case 'strokes':
  ['butt','round','square'].forEach((cap,i)=>{c.beginPath();c.lineCap=cap;c.lineWidth=3;c.moveTo(3,3+i*7);c.lineTo(12,3+i*7);c.stroke();});
  ['miter','round','bevel'].forEach((join,i)=>{c.beginPath();c.lineCap='butt';c.lineJoin=join;c.miterLimit=2;c.moveTo(18,5+i*7);c.lineTo(23,2+i*7);c.lineTo(28,5+i*7);c.stroke();});
  c.beginPath();c.lineWidth=1;c.setLineDash([2,1,3]);c.lineDashOffset=1;c.moveTo(1,22.5);c.lineTo(31,22.5);c.stroke();break;
 case 'clip-transform':
  c.translate(16,12);c.rotate(.3);c.scale(1.2,.8);c.rect(-10,-10,20,20);c.clip();c.save();
  c.beginPath();c.arc(0,0,9,0,Math.PI*2);c.clip();c.fillStyle='#2080e0';c.fillRect(-30,-30,60,60);c.restore();
  c.resetTransform();c.globalCompositeOperation='destination-out';c.fillRect(14,0,4,24);break;
 case 'linear':case 'radial':case 'conic':{
  const g=name==='linear'?c.createLinearGradient(0,0,32,0):name==='radial'?c.createRadialGradient(16,12,0,16,12,16):c.createConicGradient(0,16,12);
  g.addColorStop(0,'red');g.addColorStop(.5,'lime');g.addColorStop(1,'blue');c.fillStyle=g;c.fillRect(0,0,32,24);break;}
 case 'patterns':
  c.imageSmoothingEnabled=false;['repeat','repeat-x','repeat-y','no-repeat'].forEach((rep,i)=>{const p=c.createPattern(image(),rep);p.setTransform(new DOMMatrix().translate(2+i*8,2).scale(2));c.fillStyle=p;c.fillRect(i*8,0,8,24);});break;
 case 'images':
  c.imageSmoothingEnabled=false;c.drawImage(image(),0,0,2,2,1,1,12,12);c.imageSmoothingEnabled=true;c.drawImage(image(),0,0,2,2,17,1,12,12);
  c.putImageData(image().getContext('2d').getImageData(0,0,2,2),3,18,2,0,-1,2);break;
 case 'composites':
  modes.forEach((mode,x)=>{c.save();c.beginPath();c.rect(x,0,1,3);c.clip();c.fillStyle='rgba(60,120,180,0.6)';c.fillRect(x,0,1,2);c.globalCompositeOperation=mode;c.fillStyle='rgba(200,80,40,0.4)';c.fillRect(x,1,1,2);c.restore();});break;
 }
 out[name]=cv.toDataURL('image/png').split(',')[1];
}
document.getElementById('refs').textContent=JSON.stringify(out);
"""

def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--chromium', default='chromium')
    args = parser.parse_args()
    dest = Path(__file__).resolve().parent
    root = next(p for p in dest.parents if (p / '.git').exists())
    (root / 'work').mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='raster-oracle-', dir=root / 'work') as td:
        from websockets.asyncio.client import connect
        # CDP avoids dump-dom startup hangs in containerized Chromium builds.
        env = dict(os.environ, XDG_CONFIG_HOME=td+'/config', XDG_CACHE_HOME=td+'/cache')
        with open(Path(td) / 'browser.log', 'w') as log:
            proc = subprocess.Popen([args.chromium, '--headless', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage', '--disable-background-networking', '--disable-extensions', '--no-first-run', '--remote-debugging-address=127.0.0.1', '--remote-debugging-port=0', '--user-data-dir='+td+'/profile', 'about:blank'], stdout=subprocess.DEVNULL, stderr=log, env=env, start_new_session=True)
            try:
                port_file = Path(td) / 'profile/DevToolsActivePort'
                for _ in range(100):
                    if port_file.exists():
                        break
                    time.sleep(0.1)
                if not port_file.exists():
                    raise RuntimeError('Chromium debugging endpoint unavailable')
                port, endpoint = port_file.read_text().splitlines()[:2]
                async def capture():
                    async with connect('ws://127.0.0.1:'+port+endpoint, proxy=None, open_timeout=5, max_size=4*1024*1024) as ws:
                        request_id = 0
                        async def call(method, params, session=None):
                            nonlocal request_id
                            request_id += 1
                            request = {'id': request_id, 'method': method, 'params': params}
                            if session:
                                request['sessionId'] = session
                            await ws.send(json.dumps(request))
                            while True:
                                reply = json.loads(await asyncio.wait_for(ws.recv(), 10))
                                if reply.get('id') == request_id:
                                    if 'error' in reply:
                                        raise RuntimeError(reply['error'])
                                    return reply['result']
                        target = await call('Target.createTarget', {'url': 'about:blank'})
                        session = await call('Target.attachToTarget', {'targetId': target['targetId'], 'flatten': True})
                        code = SCENES.replace("document.getElementById('refs').textContent=JSON.stringify(out);", 'return out;')
                        font_path = dest.parents[2] / 'recorder/tests/fixtures/recorder-test.ttf'
                        font_url = 'data:font/ttf;base64,'+base64.b64encode(font_path.read_bytes()).decode('ascii')
                        preamble = 'const face=new FontFace("Recorder Test",'+json.dumps('url('+font_url+')')+');await face.load();document.fonts.add(face);'
                        result = await call('Runtime.evaluate', {'expression': '(async()=>{'+preamble+code+'})()', 'awaitPromise': True, 'returnByValue': True}, session['sessionId'])
                        if 'exceptionDetails' in result:
                            raise RuntimeError('Canvas fixture evaluation failed: '+str(result['exceptionDetails']))
                        return result['result']['value']
                refs = asyncio.run(capture())
            finally:
                os.killpg(proc.pid, signal.SIGTERM)
                proc.wait(timeout=5)
        if len(refs) != 13:
            raise RuntimeError('Incomplete oracle')
        hashes = {}
        for name, encoded in refs.items():
            data = base64.b64decode(encoded, validate=True)
            (dest / (name + '.png')).write_bytes(data)
            hashes[name + '.png'] = hashlib.sha256(data).hexdigest()
        manifest = {'generator': 'generate.py', 'browser': subprocess.check_output([args.chromium, '--version'], text=True).strip(), 'format': 'straight sRGB RGBA8 PNG, canvas.toDataURL; no screenshot', 'sha256': hashes}
        (dest / 'manifest.json').write_text(json.dumps(manifest, indent=2, sort_keys=True)+'\n')
        print(f'Generated {len(refs)} synthetic PNG references')
if __name__ == '__main__':
    main()
