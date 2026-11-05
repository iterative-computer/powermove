#!/usr/bin/env python3
"""Run the complete pipeline on a disposable, generated project (no owner data)."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import sys
from rotation import generate, UnsupportedFFmpeg

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--binary', type=Path, default=Path.home() / 'Developer/pm-target/debug/examples/pm-corpus')
p.add_argument('--compiler',type=Path,help='Explicit compiler for a relocated/frozen worker.')
p.add_argument('--worker-only', action='store_true', help='Use an independent fixture root while a real driver is active.')
p.add_argument('--legacy', action='store_true')
p.add_argument('--text', action='store_true')
p.add_argument('--rotated', action='store_true', help='Qualify generated default-path rotated pixels, then all stages (requires Mac).')
p.add_argument('--offline', action='store_true', help='Reproduce the missing-original refusal without any real media.')
p.add_argument('--effect', choices=('glow','shadow'), help='Prove generated native effect pixels and all nine stages.')
p.add_argument('--expect-refusal',action='store_true',help='Demonstrate a pre-fix capability refusal explicitly.')
args = p.parse_args()
if args.effect:
    args.text = True
if args.rotated or args.offline or args.effect:
    args.worker_only = True
with tempfile.TemporaryDirectory(prefix='pm-corpus-synthetic-') as directory:
    profile = Path(directory).resolve()
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
            try:
                for turns in (1,2,3):
                    witness = generate(profile, turns)
                    if not args.expect_refusal:
                        spec = profile/'rotation-proof.json'
                        spec.write_text(json.dumps(witness))
                        env = {**os.environ, **({'POWERMOVE_GLSL_COMPILER':str(args.compiler)} if args.compiler else {})}
                        subprocess.run([str(args.binary),'rotation-proof',str(spec)],env=env,check=True)
                source = Path(witness['rotated'])
            except UnsupportedFFmpeg as error:
                raise SystemExit('BLOCKED: '+str(error))
        project['assets']={'synthetic-media':{'id':'synthetic-media','name':'Synthetic media','kind':'video','w':320,'h':180,'dur':1,'persisted':True,**({'sourcePath':str(source)} if args.rotated else {})}}
        project['layers']=[{'id':'clip','type':'video','name':'Synthetic clip','from':0,'dur':1,'on':True,'d':{'asset':'synthetic-media','trim':0,'speed':1,'w':320,'h':180,'fit':'cover','embeddedAudio':False},'p':{'scale.x':{'v':100,'kf':[],'expr':None},'scale.y':{'v':100,'kf':[],'expr':None},'opacity':{'v':100,'kf':[],'expr':None}},'fx':[],'masks':[]}]
    if args.effect:
        project['layers'][0]['fx']=[{'id':'synthetic-effect','type':args.effect,'on':True,'p':{}}]
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
        (work/'spec.json').write_text(json.dumps({'id':'0000000000000001','library':'synthetic','synthetic_pixel_proof':bool(args.effect)}))
        env=None if args.compiler is None else {**os.environ,'POWERMOVE_GLSL_COMPILER':str(args.compiler)}
        subprocess.run([str(args.binary),'project',str(root),str(work/'spec.json'),str(work)],check=True,env=env)
        result=json.loads((work/'result.json').read_text())
        if not (args.offline or args.expect_refusal) and not all(result.get(stage)=='pass' for stage in ('open','render','play','scrub','audio','edit','save','mp4','prores')):
            details='\n'.join(p.read_text() for p in work.glob('*.private.log'))
            raise SystemExit('Synthetic worker regression failed: '+str(result)+'\n'+details)
        if args.effect and not args.expect_refusal:
            # Compare identical geometry/text with and without the effect using
            # the same frozen worker/helper and the real export children.
            baseline = json.loads(json.dumps(project))
            baseline['layers'][0]['fx'] = []
            (root/'profile/store/project.synthetic.json').write_text(json.dumps(baseline))
            baseline_work = root/'baseline'
            baseline_work.mkdir()
            subprocess.run([str(args.binary),'project',str(root),str(work/'spec.json'),str(baseline_work)],check=True,env=env)
            baseline_result = json.loads((baseline_work/'result.json').read_text())
            if not all(baseline_result.get(stage)=='pass' for stage in ('open','render','play','scrub','audio','edit','save','mp4','prores')):
                raise SystemExit('Baseline fixture failed: '+str(baseline_result))
            for name in ('preview-pixels.json','mp4-pixels.rgb','prores-pixels.rgb'):
                a,b = (work/name).read_bytes(), (baseline_work/name).read_bytes()
                if a == b:
                    raise SystemExit('Effect produced no measured pixel change: '+name)
                if name.endswith('.json'):
                    pixels = json.loads(a)
                    if not pixels or not any(max(p)>0.5 for p in pixels):
                        raise SystemExit('Blank effect preview')
                elif len(set(a)) < 2:
                    raise SystemExit('Blank effect export: '+name)
        if args.offline or args.expect_refusal:
            expected='GLSL fallback runner' if args.effect else 'rotated video' if args.rotated else 'offline_originals'
            logs=[work/(stage+'.private.log') for stage in ('render','mp4','prores')]
            if not any(log.exists() and expected in log.read_text() for log in logs):
                raise SystemExit('Expected capability refusal was not reproduced')
            print('Expected synthetic refusal reproduced; no owner media used.')
            raise SystemExit(0)
        if not all(result.get(stage)=='pass' for stage in ('open','render','play','scrub','audio','edit','save','mp4','prores')):
            details='\n'.join(p.read_text() for p in work.glob('*.private.log'))
            raise SystemExit('Synthetic worker regression failed: '+str(result)+'\n'+details)
    else:
        report = Path('/tmp/pm-corpus-synthetic-summary.json')
        command=[sys.executable, str(Path(__file__).with_name('run.py')), '--binary', str(args.binary), '--profile', directory, '--projects', directory, '--report', str(report)]
        if args.compiler:command+=['--compiler',str(args.compiler)]
        subprocess.run(command, check=True)
        result = json.loads(report.read_text())
        if not all(stage['pass'] == 1 for stage in result['totals'].values()):
            raise SystemExit('Synthetic end-to-end regression failed; see count-only report')
    print('All nine stages passed; private fixture and work copies removed on exit.')
