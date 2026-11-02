#!/usr/bin/env python3
"""Three-or-more Electron reference captures per anonymous case. Inputs are read-only."""
import argparse,json,os,shutil,tempfile
from pathlib import Path
from collections import Counter
import run
from stability import inspect_repeats


def main():
    p=argparse.ArgumentParser()
    p.add_argument('--reference-root',type=Path,required=True)
    p.add_argument('--repeats',type=int,default=3)
    p.add_argument('--case',action='append',default=[])
    p.add_argument('--store',action='store_true')
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--keep-sample',action='store_true',help='Retain only one case’s small screenshots/traces for diagnosis')
    args=p.parse_args()
    if args.repeats<3:p.error('At least three captures are required')
    if args.output.resolve().is_relative_to(run.ROOT):p.error('Private diagnostics must be outside the checkout')
    run.WORK.mkdir(mode=0o700,exist_ok=True)
    store=run.PublicStore(); rows={};totals=Counter()
    populations={'owner':run.discover(Path.home()/'Library/Application Support/Powermove/extensions'),
                 'local-corpus':run.discover(run.ROOT/'apps/desktop/e2e/fixtures/compat-corpus')}
    if args.store:populations['store']=store.catalog()
    for source,items in populations.items():
        for index,item in enumerate(items,1):
            case=f'{source}-{index:04d}'
            if args.case and case not in args.case:continue
            run.disk_check()
            with tempfile.TemporaryDirectory(prefix='repeat-',dir=run.WORK) as temp:
                root=Path(temp);input=root/'input';outputs=[]
                try:
                    sha=store.source(item,input) if source=='store' else run.copy_extension(item,input)
                    request={'version':1,'case':case,'extension':str(input),'input_sha256':sha,
                             'script_sha256':run.digest(json.dumps(run.DEFAULT_SCRIPT,sort_keys=True).encode()),
                             'script':run.DEFAULT_SCRIPT,'role':'reference'}
                    if source=='store':request['store']={'repoId':item['repoId'],'releaseId':item['latest']['id']}
                    for sample in range(args.repeats):
                        out=root/str(sample);outputs.append(out)
                        run.capture(['node',str(Path(__file__).with_name('reference.mjs')),str(args.reference_root)],request,out,90)
                    result=inspect_repeats(outputs,['active'])
                    rows[case]=result;totals['stable' if result['stable'] else 'unstable']+=1
                    if args.keep_sample:
                        args.output.parent.mkdir(parents=True,exist_ok=True)
                        target=Path(tempfile.mkdtemp(prefix='reference-sample-',dir=args.output.parent))
                        target.chmod(0o700)
                        for index,out in enumerate(outputs):
                            for name in ['trace.jsonl','active.png','interactions.json']:
                                shutil.copyfile(out/name,target/f'{index}-{name}')
                        args.keep_sample=False
                except run.Blocked as error:
                    rows[case]={'blocked':str(error)};totals[str(error)]+=1
                print(f'{case}: completed',flush=True)
            args.output.parent.mkdir(parents=True,exist_ok=True)
            fd=os.open(args.output,os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600)
            os.fchmod(fd,0o600)
            with os.fdopen(fd,'w') as report:
                report.write(json.dumps({'totals':dict(totals),'cases':rows},indent=2)+'\n')
    if args.case and set(args.case)-set(rows):p.error('Requested anonymous cases were not found')
    print(json.dumps(dict(totals)))
    return 0 if totals.get('stable',0)>0 and sum(totals.values())==totals['stable'] else 2

if __name__=='__main__':raise SystemExit(main())
