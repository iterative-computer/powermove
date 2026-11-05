#!/usr/bin/env python3
"""Clone a private corpus, run isolated native workers, emit count-only evidence.

No source is opened for writing. No shell interpolation, original app launch,
Keychain access, or extension execution. Private files live only under ROOT.
"""
import argparse
import ctypes
import errno
import hashlib
import fcntl
import json
import os
from pathlib import Path
import shutil
import signal
import re
import subprocess
import sys
import time
from uuid import uuid4

ROOT = Path('/tmp/pm-corpus')
FLOOR = 8 * 1024**3
PAUSE = Path('/tmp/pm-shots/PAUSE_LOCAL')

class LocalPause(RuntimeError):
    pass

class DiskFloor(RuntimeError):
    pass

class ShaderCompilerUnavailable(RuntimeError):
    pass
STAGES = ('open', 'render', 'play', 'scrub', 'audio', 'edit', 'save', 'mp4', 'prores')

def free_bytes():
    if sys.platform != 'darwin':
        return shutil.disk_usage('/tmp').free
    # APFS volume statistics can disagree with the shared container reserve.
    info = subprocess.run(['diskutil', 'info', '/'], capture_output=True, text=True,
                          check=True, timeout=10).stdout
    match = re.search(r'Container Free Space:.*?\((\d+) Bytes\)', info)
    if not match:
        raise RuntimeError('container_free_space_unavailable')
    return int(match[1])

def space():
    if PAUSE.exists():
        raise LocalPause('local_pause')
    if free_bytes() < FLOOR:
        raise DiskFloor('disk_floor')

def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for data in iter(lambda: f.read(1024 * 1024), b''):
            h.update(data)
    return h.hexdigest()

def inventory(path):
    out = {}
    if not path.exists():
        return out
    for base, dirs, files in os.walk(path, followlinks=False):
        for name in dirs + files:
            p = Path(base) / name
            if p.is_symlink():
                raise RuntimeError('symlink_source')
        for name in files:
            p = Path(base) / name
            s = source_stat(p)
            out[str(p.relative_to(path))] = (s.st_size, s.st_mtime_ns, digest(p))
    return out

def source_stat(path):
    result=path.stat()
    # Darwin sys/stat.h: SF_DATALESS. Reading a placeholder may hydrate the
    # original through a system daemon, outside our batch's disk controls.
    if getattr(result,'st_flags',0) & 0x40000000:
        raise RuntimeError('source_not_local')
    return result

def clone(src, dst):
    space()
    if src.is_symlink() or not src.is_file():
        raise RuntimeError('not_regular')
    before = source_stat(src)
    if free_bytes() - before.st_size < FLOOR:
        raise DiskFloor('disk_floor_payload_reserve')
    dst.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    libc = ctypes.CDLL(None, use_errno=True)
    clonefile = getattr(libc, 'clonefile', None)
    cloned = clonefile is not None and clonefile(os.fsencode(src), os.fsencode(dst), 1) == 0
    if not cloned:
        if clonefile is not None and ctypes.get_errno() not in (errno.EXDEV, errno.ENOTSUP, errno.EINVAL, errno.EPERM, errno.EACCES):
            raise OSError(ctypes.get_errno(), 'clone failed')
        # Cross-volume originals use a bounded stream, after reserving their
        # full size above the floor. Never stream a whole media library.
        created = False
        try:
            fd = os.open(src, os.O_RDONLY | os.O_NOFOLLOW)
            with os.fdopen(fd, 'rb') as source, dst.open('xb') as target:
                created = True
                for data in iter(lambda: source.read(4 * 1024 * 1024), b''):
                    if free_bytes() - len(data) < FLOOR:
                        raise DiskFloor('disk_floor_stream')
                    target.write(data)
                target.flush()
                os.fsync(target.fileno())
        except BaseException:
            if created:
                dst.unlink(missing_ok=True)
            raise
    os.chmod(dst, 0o600)
    after = src.stat()
    if (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns) or digest(src) != digest(dst):
        raise RuntimeError('source_changed')
    space()

def tree(src, dst):
    before = inventory(src)
    dst.mkdir(parents=True, exist_ok=True, mode=0o700)
    for name in before:
        clone(src / name, dst / name)
    if before != inventory(src):
        raise RuntimeError('source_changed')
    return len(before)

def freeze_tools(binary, compiler, root):
    binary=Path(binary).expanduser()
    if compiler is None:
        directory=binary.parent
        if directory.name in ('examples','deps'):
            directory=directory.parent
        compiler=directory/'pm-glsl-compiler'
    compiler=Path(compiler).expanduser()
    if not compiler.is_file() or compiler.is_symlink() or not os.access(compiler,os.X_OK):
        raise ShaderCompilerUnavailable('shader_compiler_unavailable')
    if not binary.is_file() or binary.is_symlink() or not os.access(binary,os.X_OK):
        raise RuntimeError('worker_unavailable')
    identities={'worker_sha256':digest(binary),'compiler_sha256':digest(compiler)}
    outputs=[]
    for source,folder,key in [(binary,'workers','worker_sha256'),(compiler,'compilers','compiler_sha256')]:
        directory=root/folder
        directory.mkdir(exist_ok=True,mode=0o700)
        target=directory/identities[key]
        if target.is_symlink():
            raise RuntimeError('frozen_tool_changed')
        if not target.exists():
            clone(source,target)
            target.chmod(0o700)
        if digest(target) != identities[key] or not os.access(target,os.X_OK):
            raise RuntimeError('frozen_tool_changed')
        outputs.append(target)
    return *outputs,identities

def select_records(asset_id, asset, records, profile):
    """Exact revision, preferred content key, then the shipped bare-ID fallback.

    Snapshot-origin preference applies within a key family, never ahead of it.
    A content-addressed original can be present without a legacy key manifest.
    The worker verifies its BLAKE3 before binding the captured bytes.
    """
    keys = [asset.get('storageKey'),
            'media:' + asset['fingerprint'] if asset.get('fingerprint') else None,
            asset_id]
    revision = asset.get('revision')
    for key in dict.fromkeys(k for k in keys if isinstance(k, str) and k):
        matches = [r for r in records if r['key'] == key and
                   (not isinstance(revision, str) or r.get('revision') == revision)]
        if matches:
            captured = [r for r in matches if 'snapshot' in r]
            return captured or matches
    content_hash = asset.get('contentHash')
    if isinstance(content_hash, str) and len(content_hash) == 64 and all(c in '0123456789abcdef' for c in content_hash):
        return [{'key': keys[0] or keys[1] or asset_id, 'revision': revision,
                 'source': str(profile / 'Media Store/objects' / content_hash[:2] / content_hash),
                 'blake3': content_hash}]
    return []

def captured_record(record, path):
    """A stable copy must also match the imported original's recorded bytes."""
    actual = digest(Path(path))
    metadata = record.get('value')
    if isinstance(metadata, dict) and 'sha256' in metadata:
        expected = metadata['sha256']
        if (not isinstance(expected, str) or len(expected) != 64
                or any(c not in '0123456789abcdef' for c in expected)
                or expected != actual):
            raise BatchLimit('media_record_checksum_mismatch')
    return {**record, 'path': path, 'sha256': actual}

def call(binary, args, log, timeout=180, compiler=None):
    space()
    with log.open('wb') as f:
        env=None if compiler is None else {**os.environ,'POWERMOVE_GLSL_COMPILER':str(compiler)}
        child = subprocess.Popen([str(binary), *map(str, args)], stdout=f, stderr=subprocess.STDOUT, start_new_session=True,env=env)
        deadline=time.monotonic()+timeout
        try:
            while True:
                space()
                remaining=deadline-time.monotonic()
                if remaining<=0:
                    os.killpg(child.pid,signal.SIGKILL)
                    child.wait()
                    return 124
                try:
                    return child.wait(timeout=min(1,remaining))
                except subprocess.TimeoutExpired:
                    continue
        except BaseException:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
            raise

def classify(text):
    # Never copy arbitrary error strings into the public report.
    for needle, code in (
        ('ambiguous_media', 'ambiguous_media'), ('media_content_hash_mismatch', 'media_content_hash_mismatch'),
        ('AsCopy(ChangedOnDisk)', 'changed_file_association'), ('rotated video', 'rotated_video_capability'), ('no definition for effect', 'dynamic_effect_dependency'), ('no definition for transition', 'dynamic_transition_dependency'), ('roundtrip_mismatch', 'roundtrip_mismatch'), ('offline', 'offline_media'),
        ('GLSL fallback runner', 'glsl_fallback_capability'),
        ('history', 'history'), ('font', 'font'), ('shader', 'shader'),
        ('frame_timeout', 'frame_timeout'), ('decode_failed', 'decode_failed'),
        ('playhead_did_not_advance', 'playhead_stalled'), ('MCP rejected', 'mcp_rejected'),
        ('disk_floor', 'disk_floor'), ('composition', 'composition'),
        ('panic', 'panic'), ('export', 'export_failed'),
    ):
        if needle.lower() in text.lower():
            return code
    return 'unclassified'

def gate_pass(summary):
    return (summary.get('complete', False)
            and summary.get('discovered') == summary.get('full_discovered')
            and not summary.get('mixed_provenance', False)
            and not summary.get('setup_failures') and bool(summary.get('projects'))
            and all(not p.get('capture_classes') and all(v == 'pass' for v in p['stages'].values()) for p in summary['projects']))

def archive_logs(batch, destination):
    # Rechecking an ID replaces its diagnostics; do not mix earlier tool runs.
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True, mode=0o700)
    for log in batch.glob('*.private.log'):
        shutil.copyfile(log, destination / log.name)
    descriptor=batch/'asset-request.private.json'
    if descriptor.exists():
        shutil.copyfile(descriptor,destination/descriptor.name)

def interrupted_result(public, batch, used, capture_errors):
    try:
        state=json.loads((batch/'result.json').read_text())
    except (OSError,ValueError):
        state={}
    public['stages']={s:state.get(s) if state.get(s) in ('pass','fail') else 'blocked' for s in STAGES}
    for stage in STAGES:
        log=batch/(stage+'.private.log')
        if public['stages'][stage]=='fail' and log.exists():
            code=classify(log.read_text(errors='replace'))
            if code=='disk_floor':
                public['stages'][stage]='blocked'
            public['failures'].append({'stage':stage,'class':code,
                'category':'perf' if code in ('disk_floor','timeout','frame_timeout') else 'export' if stage in ('mp4','prores') else 'render' if stage in ('play','scrub') else stage})
    unfinished=[s for s in STAGES if public['stages'][s]=='blocked']
    stage=state.get('running')
    if stage not in unfinished:
        stage=unfinished[0] if unfinished else None
    if stage:
        public['failures'].append({'stage':stage,'class':'disk_floor','category':'perf'})
    if capture_errors:
        public['capture_classes']=sorted(set(capture_errors))
        for failure in public['failures']:
            if failure['class']=='offline_media':
                public['stages'][failure['stage']]='blocked'
    if state.get('open_mode')=='as-copy':
        public['open_mode']='as-copy'
    public['batch_bytes']=used
    return public

class BatchLimit(RuntimeError):
    pass

def run(args):
    profile = args.profile.expanduser().resolve()
    projects_only = getattr(args, 'projects_only', False)
    target=ROOT.resolve()
    if target == profile or target.is_relative_to(profile) or profile.is_relative_to(target):
        raise RuntimeError('source_destination_overlap')
    if ROOT.exists():
        owner = json.loads((ROOT / '.owner.json').read_text())
        if not args.resume_snapshot or ROOT.is_symlink() or owner['uid'] != os.getuid() or owner['profile'] != str(profile) or owner.get('projects_only', False) != projects_only:
            raise RuntimeError('foreign_or_existing_snapshot')
    else:
        ROOT.mkdir(mode=0o700)
        owner={'uid':os.getuid(),'profile':str(profile),'run_id':uuid4().hex,'projects_only':projects_only}
        (ROOT / '.owner.json').write_text(json.dumps(owner))
    summary = {'version': 2, 'snapshot_mode': 'per-project batches; metadata snapshot with deferred IndexedDB payloads',
               'batch_limit_bytes': args.batch_gib * 1024**3, 'floor_gib': 6,
               'projects': [], 'p2': [], 'setup_failures': [], 'snapshot': {}}
    previous=None
    if args.resume_results and args.report.exists():
        previous=json.loads(args.report.read_text())
        summary['projects']=previous['projects']
        summary['p2']=previous['p2']
        summary['setup_failures']=previous['setup_failures']
    def report():
        summary['totals'] = {s: {state: sum(p['stages'][s] == state for p in summary['projects']) for state in ('pass', 'fail', 'blocked')} for s in STAGES}
        summary['complete'] = len(summary['projects']) + len(summary['p2']) == summary.get('discovered', -1)
        summary['gate_pass'] = gate_pass(summary)
        args.report.write_text(json.dumps(summary, indent=2))
    try:
        space()
        if args.resume_snapshot and (ROOT/'batch').exists():
            shutil.rmtree(ROOT/'batch')  # Restart only our interrupted private batch, after pause ends.
        if not owner.get('run_id'):
            owner['run_id']=uuid4().hex
            (ROOT / '.owner.json').write_text(json.dumps(owner))
        summary['snapshot_run_id']=owner['run_id']
        summary['runner_sha256']=digest(Path(__file__))
        # Keep this run independent of shared-target rebuilds or cache cleanup.
        args.binary,args.compiler,identities=freeze_tools(args.binary,getattr(args,'compiler',None),ROOT)
        summary.update(identities)
        if getattr(args, 'synthetic_preflight', False):
            if call(args.binary, ['shader-info', ROOT/'shaders.private.json'], ROOT/'shaders.private.log', timeout=args.timeout, compiler=args.compiler):
                raise RuntimeError('shader_preflight_failed')
            for effect in ('glow', 'shadow'):
                if call(sys.executable, [Path(__file__).with_name('synthetic.py'), '--binary', args.binary,
                        '--compiler', args.compiler, '--effect', effect], ROOT/(effect+'.private.log'),
                        timeout=args.timeout, compiler=args.compiler):
                    raise RuntimeError('synthetic_pixel_preflight_failed')

        summary['mixed_provenance']=bool(previous and (previous.get('runner_sha256') != summary['runner_sha256'] or previous.get('worker_sha256') != summary['worker_sha256'] or previous.get('compiler_sha256') != summary['compiler_sha256'] or previous.get('snapshot_run_id') != summary['snapshot_run_id'] or previous.get('mixed_provenance')))
        if not (ROOT / 'profile/store').exists():
            if projects_only:
                (ROOT / 'profile/store').mkdir(parents=True, mode=0o700)
            else:
                tree(profile / 'store', ROOT / 'profile/store')
        records = []
        for family in (() if projects_only else ('Index', 'keys')):
            source = profile / 'Media Store' / family
            target = ROOT / 'profile/Media Store' / family
            if source.is_dir() and not target.exists():
                tree(source, target)
            for manifest in sorted(target.glob('*.json')):
                r = json.loads(manifest.read_text())
                if 'sourcePath' in r:
                    records.append({'key': r['key'], 'revision': r.get('revision'), 'source': r['sourcePath'], 'size': r.get('size', 0), 'value': r})
                elif len(r.get('hash', '')) == 64 and all(c in '0123456789abcdef' for c in r['hash']):
                    h = r['hash']
                    records.append({'key': r['key'], 'revision': r.get('revision'), 'source': str(profile / 'Media Store/objects' / h[:2] / h), 'size': r.get('size', 0), 'value': r, 'blake3': h})
        host_count = len(records)
        for i, db in enumerate([] if projects_only else sorted((profile / 'IndexedDB').glob('*.indexeddb.leveldb'))):
            target = ROOT / f'indexeddb-{i}'
            blobs = db.with_name(db.name.removesuffix('.leveldb') + '.blob')
            if not target.exists():
                code = call(args.binary, ['snapshot', db, blobs, target], ROOT / f'snapshot-{i}.private.log', timeout=120,compiler=args.compiler)
                if code:
                    summary['setup_failures'].append({'stage': 'snapshot', 'id': hashlib.sha256(db.name.encode()).hexdigest()[:16], 'class': 'reader_failed'})
                    continue
            records.extend(json.loads((target / 'records.json').read_text()))
        summary['snapshot'] = {'host_records': host_count, 'indexeddb_records': len(records) - host_count,
                               'store_files': len(list((ROOT / 'profile/store').glob('*')))}
        if not ((ROOT/'inventory.json').exists() and owner.get('inventory_worker_sha256') == summary['worker_sha256']):
            if call(args.binary, ['inventory', ROOT], ROOT / 'inventory.private.log', timeout=180,compiler=args.compiler):
                raise RuntimeError('library_inventory_failed')
            owner['inventory_worker_sha256']=summary['worker_sha256']
            (ROOT/'.owner.json').write_text(json.dumps(owner))
        inv = json.loads((ROOT / 'inventory.json').read_text())
        rows = inv['projects']
        summary['library_slots'] = len(rows)
        files = set(inv['associated_files'])
        discovery_path=ROOT/'discovery.private.json'
        discovery_key={'roots':[str(p.expanduser()) for p in args.projects],
                       'extra_sha256':digest(args.extra_files) if args.extra_files else None}
        cached=json.loads(discovery_path.read_text()) if discovery_path.exists() else None
        if args.resume_snapshot and cached and cached.get('key') == discovery_key:
            files.update(cached['files'])
            for failure in cached['failures']:
                if failure not in summary['setup_failures']:
                    summary['setup_failures'].append(failure)
        else:
            for directory in args.projects:
                directory = directory.expanduser()
                if not directory.is_dir():
                    continue
                try:
                    found = subprocess.run(['rg', '--files', '--no-ignore', '-g', '*.pmv', '-g', '!**/*.app/**', '-g', '!**/node_modules/**', '-g', '!**/target/**', '-g', '!**/vendor/**', str(directory)], capture_output=True, timeout=30)
                    files.update(os.fsdecode(p) for p in found.stdout.splitlines())
                    if found.returncode not in (0, 1):
                        summary['setup_failures'].append({'stage': 'discovery', 'class': 'unreadable_search_root'})
                except subprocess.TimeoutExpired as error:
                    files.update(os.fsdecode(p) for p in (error.stdout or b'').splitlines())
                    summary['setup_failures'].append({'stage': 'discovery', 'class': 'discovery_timeout'})
                    # Indexed discovery can recover known local PMVs when cloud
                    # directory enumeration stalls. Keep the timeout visible because
                    # an index cannot certify the absence of unindexed files.
                    try:
                        indexed=subprocess.run(['mdfind','-onlyin',str(directory),'kMDItemFSName == "*.pmv"cd'],capture_output=True,text=True,timeout=10)
                        if indexed.returncode==0:
                            files.update(x for x in indexed.stdout.splitlines() if x.lower().endswith('.pmv') and not any(p in Path(x).parts for p in ('node_modules','target','vendor','.git')))
                    except subprocess.TimeoutExpired:
                        pass
            if args.extra_files:
                files.update(json.loads(args.extra_files.read_text()))
            discovery_path.write_text(json.dumps({'key':discovery_key,'files':sorted(files),
                'failures':[f for f in summary['setup_failures'] if f['stage']=='discovery']}))
        rows.extend({'id': hashlib.sha256(os.fsencode(src)).hexdigest()[:16], 'file_source': src} for src in sorted(files))
        summary['file_entries'] = len(files)
        summary['full_discovered'] = len(rows)
        if args.only_ids:
            wanted = set(args.only_ids.split(','))
            rows = [r for r in rows if r['id'] in wanted]
        summary['discovered'] = len(rows)
        processed={p['id'] for p in summary['projects']} | set(summary['p2'])
        for row in sorted(rows, key=lambda r: r['id']):
            if row['id'] in processed:
                continue
            if row.get('extension'):
                summary['p2'].append(row['id'])
                report()
                continue
            space()
            batch = ROOT / 'batch'
            batch.mkdir(mode=0o700)
            public = {'id': row['id'], 'stages': {s: 'blocked' for s in STAGES}, 'failures': []}
            used = 0
            linked, selected, copied = {}, [], {}
            rejected_keys, rejected_paths = set(), set()
            capture_errors = []
            def capture(src, size=None, record=None):
                nonlocal used
                identity = src if src else f"{record['snapshot']}:{record['database']}:{record['number']}"
                if identity in copied:
                    return copied[identity]
                if src:
                    try:
                        info = subprocess.run([sys.executable, __file__, '--stat-one', src], capture_output=True, text=True, timeout=5)
                        if info.returncode:
                            raise BatchLimit('source_not_local' if 'source_not_local' in info.stderr else 'source_unavailable')
                        size = int(info.stdout)
                    except (subprocess.TimeoutExpired, ValueError):
                        raise BatchLimit('source_probe_timeout')
                size = size or 0
                if used + size > args.batch_gib * 1024**3:
                    raise BatchLimit('batch_media_over_budget')
                space()
                dest = batch / ('media-' + hashlib.sha256(identity.encode()).hexdigest()[:16] + (Path(src).suffix if src else '.bin'))
                if src:
                    # Copy/hash source reads are isolated too: cloud placeholders may block.
                    code = call(sys.executable, [__file__, '--clone-one', src, dest], batch / 'capture.private.log', timeout=30)
                    if code == 124:
                        raise BatchLimit('copy_timeout')
                    if code:
                        error = (batch / 'capture.private.log').read_text(errors='replace')
                        if 'disk_floor' in error:
                            space()
                            raise BatchLimit('batch_disk_capacity')
                        if 'local_pause' in error:
                            raise LocalPause('local_pause')
                        raise BatchLimit('copy_failed')
                else:
                    code = call(args.binary, ['blob', record['snapshot'], record['database'], record['number'], dest], batch / ('blob-' + hashlib.sha256(identity.encode()).hexdigest()[:16] + '.private.log'), timeout=60,compiler=args.compiler)
                    if code:
                        text = (batch / ('blob-' + hashlib.sha256(identity.encode()).hexdigest()[:16] + '.private.log')).read_text(errors='replace')
                        if 'disk_floor' in text:
                            space()
                            raise BatchLimit('batch_disk_capacity')
                        raise BatchLimit('snapshot_blob_changed_or_unavailable')
                space()
                used += size
                copied[identity] = str(dest)
                return str(dest)
            try:
                if 'file_source' in row:
                    try:
                        row['file'] = capture(row['file_source'])
                        code = call(args.binary, ['file-info', row['file'], batch / 'info.private.json'], batch / 'info.private.log', timeout=60,compiler=args.compiler)
                        if code:
                            raise BatchLimit('project_file_reader_failed')
                        info = json.loads((batch / 'info.private.json').read_text())
                        row.update(info)
                    except BatchLimit as e:
                        public['stages']['open'] = 'blocked' if str(e) in ('batch_disk_capacity','batch_media_over_budget','source_not_local') else 'fail'
                        public['failures'].append({'stage': 'open', 'category': 'open', 'class': str(e)})
                        summary['projects'].append(public)
                        report()
                        continue
                    if row.get('extension'):
                        summary['p2'].append(row['id'])
                        report()
                        continue
                for src in row.get('associated', []):
                    try:
                        linked[src] = capture(src)
                    except BatchLimit as error:
                        capture_errors.append(str(error))  # The independent slot remains usable offline.
                assets = row.get('assets') or {}
                for asset_id, a in assets.items():
                    if asset_id in row.get('embedded', []):
                        continue
                    matches = select_records(asset_id, a, records, profile)
                    selected_before = len(selected)
                    for r in matches:
                        try:
                            path = capture(r.get('source'), r.get('size'), r)
                            selected.append(captured_record(r, path))
                        except BatchLimit as e:
                            capture_errors.append(str(e))
                            if str(e) == 'media_record_checksum_mismatch':
                                rejected_keys.add(r['key'])
                                rejected_paths.update(src for src in (
                                    r.get('source'), a.get('sourcePath'), a.get('path'),
                                    (a.get('link') or {}).get('path')) if isinstance(src, str))
                    if len(selected) > selected_before and a.get('storage') != 'linked' and not a.get('link'):
                        continue
                    for src in (a.get('sourcePath'), a.get('path'), (a.get('link') or {}).get('path')):
                        if isinstance(src, str) and os.path.isabs(src):
                            try:
                                linked[src] = capture(src)
                            except BatchLimit as e:
                                capture_errors.append(str(e))
                # Detached file associations cannot cause writes to original PMVs.
                # A failed recorded identity cannot be bypassed by another
                # asset's direct-path mapping or a second origin with this key.
                (ROOT / 'media.json').write_text(json.dumps({
                    'records': [r for r in selected if r['key'] not in rejected_keys],
                    'linked': {src: path for src, path in linked.items() if src not in rejected_paths},
                }))
                (batch / 'spec.private.json').write_text(json.dumps(row))
                code = call(args.binary, ['project', ROOT, batch / 'spec.private.json', batch], batch / 'worker.private.log', timeout=args.timeout,compiler=args.compiler)
                state = json.loads((batch / 'result.json').read_text()) if (batch / 'result.json').exists() else {}
                if state.get('extension') == 'p2':
                    summary['p2'].append(row['id'])
                    report()
                    continue
                running = state.pop('running', 'open')
                public['stages'] = {s: state.get(s, 'blocked') for s in STAGES}
                if code:
                    stage = running if running in STAGES else 'open'
                    public['stages'][stage] = 'fail'
                    public['failures'].append({'stage': stage, 'class': 'timeout' if code == 124 else classify((batch / 'worker.private.log').read_text(errors='replace'))})
                for stage in STAGES:
                    log = batch / (stage + '.private.log')
                    if state.get(stage) == 'fail' and log.exists():
                        public['failures'].append({'stage': stage, 'class': classify(log.read_text(errors='replace'))})
                if capture_errors:
                    public['capture_classes'] = sorted(set(capture_errors))
                    # An unmaterialized original is coverage/resource evidence, not
                    # proof that the renderer itself is broken.
                    for stage in ('render', 'audio', 'mp4', 'prores'):
                        if public['stages'][stage] == 'fail' and any(f['stage'] == stage and f['class'] == 'offline_media' for f in public['failures']):
                            public['stages'][stage] = 'blocked'
                for failure in public['failures']:
                    stage = failure['stage']
                    if failure['class'] == 'disk_floor':
                        public['stages'][stage] = 'blocked'
                    failure['category'] = 'perf' if failure['class'] in ('timeout', 'frame_timeout') else ('export' if stage in ('mp4', 'prores') else 'render' if stage in ('play', 'scrub') else stage)
                if state.get('open_mode') == 'as-copy':
                    public['open_mode'] = 'as-copy'
                public['batch_bytes'] = used
                summary['projects'].append(public)
                # Keep only private error logs for brief triage; batch media is deleted.
                failures = ROOT / 'failures' / row['id']
                if public['failures']:
                    archive_logs(batch, failures)
                report()
                print(json.dumps({'completed': len(summary['projects']), 'p2': len(summary['p2']), 'discovered': len(rows), 'id': row['id'], 'stages': public['stages'], 'batch_bytes': used}), flush=True)
            except DiskFloor:
                # Keep completed stage evidence before deleting the stopped batch.
                # A later resume can cover other projects instead of retrying the
                # same resource-heavy project indefinitely.
                summary['projects'].append(interrupted_result(public,batch,used,capture_errors))
                report()
                raise
            finally:
                if PAUSE.exists() or isinstance(sys.exc_info()[1],LocalPause):
                    args.retain_for_debug=True
                else:
                    shutil.rmtree(batch)
            # Stop before admitting another batch below the owner's floor.
            space()
        report()
    except Exception as error:
        if isinstance(error,(LocalPause,DiskFloor)):
            args.retain_for_debug=True
        summary['setup_failures'].append({'stage': 'setup', 'class': 'local_pause' if isinstance(error,LocalPause) else 'shader_compiler_unavailable' if isinstance(error,ShaderCompilerUnavailable) else 'disk_floor' if isinstance(error,DiskFloor) or free_bytes() < FLOOR else str(error) if str(error) in ('shader_preflight_failed','synthetic_pixel_preflight_failed','frozen_tool_changed','worker_unavailable') else 'runner_setup_failed'})
        report()
        raise
    finally:
        if not args.retain_for_debug and not PAUSE.exists():
            shutil.rmtree(ROOT)

def main():
    if len(sys.argv) == 3 and sys.argv[1] == '--stat-one':
        print(source_stat(Path(sys.argv[2])).st_size)
        return
    if len(sys.argv) == 4 and sys.argv[1] == '--clone-one':
        clone(Path(sys.argv[2]), Path(sys.argv[3]))
        return
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, default=Path.home() / 'Developer/pm-target/debug/examples/pm-corpus')
    p.add_argument('--compiler',type=Path,help='GLSL compiler helper; defaults to pm-glsl-compiler next to the build output.')
    p.add_argument('--synthetic-preflight', action='store_true', help='Prove real builtin shader translation and glow/shadow preview/export pixels with this frozen tool pair before capture (Mac).')
    p.add_argument('--profile', type=Path, default=Path.home() / 'Library/Application Support/Powermove')
    p.add_argument('--projects', type=Path, action='append')
    p.add_argument('--projects-only', action='store_true', help='Use an empty library and no profile media stores; discover only the supplied project folders.')
    p.add_argument('--extra-files', type=Path, help='Private JSON list of additional PMV source paths, for indexed discovery or targeted rechecks.')
    p.add_argument('--report', type=Path, default=Path('/tmp/pm-corpus-summary.json'))
    p.add_argument('--timeout', type=int, default=120)
    p.add_argument('--resume-results', action='store_true', help='Keep recorded results and continue unprocessed IDs from the owned snapshot.')
    p.add_argument('--only-ids', help='Comma-separated anonymized IDs for a targeted recheck.')
    p.add_argument('--batch-gib', type=float, default=2.0)
    p.add_argument('--resume-snapshot', action='store_true')
    p.add_argument('--retain-for-debug', action='store_true', help='Keep metadata and private diagnostic logs, never completed batch media; remove /tmp/pm-corpus when done.')
    args = p.parse_args()
    if args.projects_only and not args.projects:
        p.error('--projects-only requires at least one --projects folder')
    args.projects = args.projects or [Path.home() / d for d in ('Documents', 'Desktop', 'Downloads', 'Movies')]
    if not 0 < args.batch_gib <= 2:
        p.error('--batch-gib must be greater than zero and at most 2')
    if args.report.parent.resolve() != Path('/tmp').resolve():
        p.error('--report must be directly under /tmp')
    with open('/tmp/pm-corpus-run.lock', 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        run(args)

if __name__ == '__main__':
    main()
