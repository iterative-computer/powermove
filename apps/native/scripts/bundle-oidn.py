#!/usr/bin/env python3
"""Relocate an OIDN installation into a native app; no downloads or builds."""
import argparse, os, plistlib, re, shutil, subprocess
from pathlib import Path

def output(*args):
    return subprocess.check_output(list(map(str,args)),text=True)
def dylibs(path):
    return [line.strip().split(' (compatibility version')[0] for line in output('otool','-L',path).splitlines()[1:]]
def version(value):
    parts=tuple(map(int,value.split('.')))
    return parts+(0,)*(3-len(parts))
def rpaths(path):
    return set(re.findall(r'\bpath (.*?) \(offset',output('otool','-l',path)))
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('app',type=Path)
    parser.add_argument('--oidn-dir',type=Path,default=os.environ.get('OIDN_DIR'))
    args=parser.parse_args()
    if args.oidn_dir is None: parser.error('--oidn-dir or OIDN_DIR is required')
    source=args.oidn_dir.resolve(strict=True)/'lib'
    app=args.app.resolve(strict=True);contents=app/'Contents'
    info=plistlib.loads((contents/'Info.plist').read_bytes())
    executable=contents/'MacOS'/info['CFBundleExecutable']
    notices=args.oidn_dir/'doc'
    if not notices.is_dir(): raise RuntimeError('OIDN license directory missing')
    frameworks=contents/'Frameworks';frameworks.mkdir(exist_ok=True)
    libraries=sorted({p for pattern in ['libOpenImageDenoise*.dylib','libtbb*.dylib'] for p in source.glob(pattern)})
    if not (source/'libOpenImageDenoise.dylib').is_file(): raise RuntimeError('OIDN library missing')
    # These libraries and aliases must travel together; no Homebrew runtime.
    for component in ['core','device_metal','device_cpu']:
        if not any(p.name.startswith('libOpenImageDenoise_'+component+'.') for p in libraries):
            raise RuntimeError('OIDN '+component+' missing')
    if not any(p.name.startswith('libtbb.') for p in libraries): raise RuntimeError('OIDN TBB missing')
    names={p.name for p in libraries}
    minimum=version(info.get('LSMinimumSystemVersion','14.0'))
    # Reject an incompatible runtime before modifying the app's executable.
    # In particular, a Homebrew TBB built for macOS 27 is not a CPU fallback
    # for this app's macOS-14 deployment target.
    real=[p for p in libraries if not p.is_symlink()]
    if len(real)!=5: raise RuntimeError('Expected five OIDN/Metal/CPU/TBB dylibs')
    for lib in real:
        versions=re.findall(r'\bminos ([0-9.]+)',output('otool','-l',lib))
        if not versions or any(version(v)>minimum for v in versions):
            raise RuntimeError('OIDN runtime minimum exceeds app target: '+lib.name+' '+str(versions))
    for lib in libraries:
        target=frameworks/lib.name
        if lib.is_symlink():
            relative=os.readlink(lib)
            if Path(relative).is_absolute() or '/' in relative or relative not in names:
                raise RuntimeError('OIDN alias escapes bundle: '+str(lib))
            if target.is_symlink() or target.exists(): target.unlink()
            target.symlink_to(relative)
        else: shutil.copy2(lib,target)
    for target in [executable,*[frameworks/p.name for p in libraries if not p.is_symlink()]]:
        deps=dylibs(target)
        for dep in deps:
            filename=Path(dep).name
            if filename in names:
                relocated='@rpath/'+filename
                if dep!=relocated: subprocess.run(['install_name_tool','-change',dep,relocated,str(target)],check=True)
        if target != executable:
            subprocess.run(['install_name_tool','-id','@rpath/'+target.name,str(target)],check=True)
            for dep in dylibs(target):
                if dep.startswith('/') and not dep.startswith(('/usr/lib/','/System/Library/')):
                    raise RuntimeError('Unbundled OIDN dependency '+dep)
            versions=re.findall(r'\bminos ([0-9.]+)',output('otool','-l',target))
            if not versions or any(version(v)>minimum for v in versions):
                raise RuntimeError('Bundled OIDN minimum exceeds app target: '+target.name)
        for path in rpaths(target):
            # Libraries travel together; the app retains unrelated development
            # runtime paths (e.g. FFmpeg) but never an OIDN installation fallback.
            oidn_path=path.startswith(str(args.oidn_dir.resolve())) or 'oidn' in path.lower() or 'open-image-denoise' in path.lower()
            if path.startswith('/') and (target!=executable or oidn_path):
                subprocess.run(['install_name_tool','-delete_rpath',path,str(target)],check=True)
        if target==executable and '@executable_path/../Frameworks' not in rpaths(target):
            subprocess.run(['install_name_tool','-add_rpath','@executable_path/../Frameworks',str(target)],check=True)
        subprocess.run(['codesign','--force','--sign','-',str(target)],check=True)
    # Preserve upstream and dependency notices alongside the binary resources.
    destination=contents/'Resources'/'oidn-notices';destination.mkdir(parents=True,exist_ok=True)
    shutil.copytree(notices,destination,dirs_exist_ok=True)
    print('Bundled',len([p for p in libraries if not p.is_symlink()]),'OIDN/CPU/Metal/TBB libraries in',frameworks)
if __name__=='__main__': main()
