#!/usr/bin/env python3
"""Run the complete pipeline on a disposable, generated project (no owner data)."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--binary', type=Path, default=Path.home() / 'Developer/pm-target/debug/examples/pm-corpus')
p.add_argument('--worker-only', action='store_true', help='Use an independent fixture root while a real driver is active.')
p.add_argument('--legacy', action='store_true')
p.add_argument('--text', action='store_true')
p.add_argument('--rotated', action='store_true', help='Reproduce the rotated-video capability refusal with generated media.')
p.add_argument('--offline', action='store_true', help='Reproduce the missing-original refusal without any real media.')
args = p.parse_args()
if args.rotated or args.offline:
    args.worker_only = True
with tempfile.TemporaryDirectory(prefix='pm-corpus-synthetic-') as directory:
    profile = Path(directory)
    (profile / 'store').mkdir()
    project = {
        'id': 'synthetic', 'name': 'Synthetic corpus project', 'compId': 'C1',
        'compName': 'Synthetic composition', 'w': 640, 'h': 360, 'fps': 30,
        'dur': 3, 'bg': '#202020', 'layers': [], 'comps': {}, 'assets': {},
        'markers': [], 'params': {}, 'work': [0, 3], 'revision': 0,
    }
    if args.legacy:
        project.pop('compId')
        project.pop('compName')
    if args.text:
        project['layers']=[{'id':'title','type':'text','name':'Synthetic title','from':0,'dur':3,'on':True,'d':{'text':'Synthetic visible text','font':'Helvetica','size':32,'color':'#FFFFFF'},'p':{'position.x':{'v':160,'kf':[],'expr':None},'position.y':{'v':90,'kf':[],'expr':None},'scale.x':{'v':100,'kf':[],'expr':None},'scale.y':{'v':100,'kf':[],'expr':None},'opacity':{'v':100,'kf':[],'expr':None}},'fx':[],'masks':[]}]
    if args.rotated or args.offline:
        source=profile/'rotated.mp4'
        if args.rotated:
            plain=profile/'plain.mp4'
            subprocess.run(['ffmpeg','-v','error','-f','lavfi','-i','testsrc2=size=320x180:rate=30:duration=1','-c:v','libx264','-pix_fmt','yuv420p',str(plain)],check=True)
            subprocess.run(['ffmpeg','-v','error','-display_rotation','90','-i',str(plain),'-c','copy',str(source)],check=True)
        if args.rotated:
            probe=subprocess.run(['ffprobe','-v','error','-show_entries','stream_side_data=rotation','-of','json',str(source)],capture_output=True,text=True,check=True)
            if 'rotation' not in probe.stdout:
                raise SystemExit('FFmpeg did not attach a display rotation')
        project['assets']={'synthetic-media':{'id':'synthetic-media','name':'Synthetic media','kind':'video','w':320,'h':180,'dur':1,'persisted':True,**({'sourcePath':str(source)} if args.rotated else {})}}
        project['layers']=[{'id':'clip','type':'video','name':'Synthetic clip','from':0,'dur':1,'on':True,'d':{'asset':'synthetic-media','trim':0,'speed':1,'w':320,'h':180,'fit':'cover','embeddedAudio':False},'p':{'scale.x':{'v':100,'kf':[],'expr':None},'scale.y':{'v':100,'kf':[],'expr':None},'opacity':{'v':100,'kf':[],'expr':None}},'fx':[],'masks':[]}]
    (profile / 'store/project.synthetic.json').write_text(json.dumps(project))
    (profile / 'store/projects.json').write_text(json.dumps([{'id': 'synthetic', 'name': 'Synthetic'}]))
    if args.worker_only:
        root=profile/'worker-root'
        root.mkdir()
        (root/'profile').mkdir()
        import shutil
        shutil.copytree(profile/'store',root/'profile/store')
        linked={}
        if args.rotated:
            shutil.copyfile(source,root/'rotated.mp4')
            linked[str(source)]=str(root/'rotated.mp4')
        (root/'media.json').write_text(json.dumps({'records':[],'linked':linked}))
        work=root/'batch'
        work.mkdir()
        (work/'spec.json').write_text(json.dumps({'id':'0000000000000001','library':'synthetic'}))
        subprocess.run([str(args.binary),'project',str(root),str(work/'spec.json'),str(work)],check=True)
        result=json.loads((work/'result.json').read_text())
        if args.rotated or args.offline:
            expected='rotated video' if args.rotated else 'offline_originals'
            log=work/'render.private.log'
            if not log.exists() or expected not in log.read_text():
                raise SystemExit('Expected capability refusal was not reproduced')
            print('Expected synthetic refusal reproduced; no owner media used.')
            raise SystemExit(0)
        if not all(result.get(stage)=='pass' for stage in ('open','render','play','scrub','audio','edit','save','mp4','prores')):
            raise SystemExit('Synthetic worker regression failed: '+str(result))
    else:
        report = Path('/tmp/pm-corpus-synthetic-summary.json')
        subprocess.run(['python3', str(Path(__file__).with_name('run.py')), '--binary', str(args.binary), '--profile', directory, '--projects', directory, '--report', str(report)], check=True)
        result = json.loads(report.read_text())
        if not all(stage['pass'] == 1 for stage in result['totals'].values()):
            raise SystemExit('Synthetic end-to-end regression failed; see count-only report')
    print('All nine stages passed; private fixture and work copies removed on exit.')
