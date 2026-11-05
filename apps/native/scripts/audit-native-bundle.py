#!/usr/bin/env python3
"""Read-only native resource/Mach-O audit. Python 3.9+; never signs or installs.

The owner supplies a complete, reviewed resource manifest. No guessed list can
certify that runtime-discovered fonts, V8 snapshots, plugins or helpers exist.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

MACHO = {b'\xfe\xed\xfa\xce', b'\xce\xfa\xed\xfe', b'\xfe\xed\xfa\xcf',
         b'\xcf\xfa\xed\xfe', b'\xca\xfe\xba\xbe', b'\xbe\xba\xfe\xca',
         b'\xca\xfe\xba\xbf', b'\xbf\xba\xfe\xca'}
ROLES = {'engine', 'v8', 'extensions', 'agents', 'fonts', 'sparkle'}
SYSTEM = ('/System/Library/', '/usr/lib/')


def run(args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def inside(path, root):
    try:
        path.relative_to(root)
        return True
    except ValueError:
        return False


def relative(value):
    path = Path(value)
    if path.is_absolute() or not path.parts or '..' in path.parts or str(path) == '.':
        raise ValueError('unsafe bundle-relative resource path')
    return path


def version(text):
    parts = text.split('.')
    if not 1 <= len(parts) <= 3 or not all(p.isdecimal() for p in parts):
        raise ValueError('invalid deployment version')
    return tuple(map(int, parts)) + (0,) * (3 - len(parts))


def commands(text):
    """Parse otool -l per architecture; keep only load-time commands."""
    result = {'rpaths': [], 'dependencies': [], 'minimum': None}
    blocks = re.split(r'Load command \d+\s*\n', text)[1:]
    for block in blocks:
        match = re.search(r'^\s*cmd (LC_\w+)\s*$', block, re.M)
        if not match:
            continue
        kind = match.group(1)
        if kind == 'LC_RPATH':
            match = re.search(r'^\s*path (.+) \(offset \d+\)\s*$', block, re.M)
            if not match:
                raise ValueError('malformed LC_RPATH')
            result['rpaths'].append(match.group(1))
        elif kind in ('LC_LOAD_DYLIB', 'LC_LOAD_WEAK_DYLIB', 'LC_REEXPORT_DYLIB', 'LC_LOAD_UPWARD_DYLIB', 'LC_LAZY_LOAD_DYLIB'):
            match = re.search(r'^\s*name (.+) \(offset \d+\)\s*$', block, re.M)
            if not match:
                raise ValueError('malformed dylib load')
            # A missing weak library may affect functionality; keep it visible.
            result['dependencies'].append(match.group(1))
        elif kind in ('LC_BUILD_VERSION', 'LC_VERSION_MIN_MACOSX'):
            if kind == 'LC_BUILD_VERSION' and not re.search(r'^\s*platform (?:1|macos)\s*$', block, re.M | re.I):
                raise ValueError('non-macOS Mach-O platform')
            match = re.search(r'^\s*(?:minos|version) ([\d.]+)\s*$', block, re.M)
            if not match:
                raise ValueError('missing deployment version')
            value = match.group(1)
            if result['minimum'] and result['minimum'] != value:
                raise ValueError('conflicting deployment commands')
            result['minimum'] = value
    if not result['minimum']:
        raise ValueError('Mach-O has no macOS deployment command')
    return result


def audit(bundle, manifest, runner=run):
    root = Path(bundle).resolve(strict=True)
    issues = []
    if manifest.get('schema') != 1 or manifest.get('reviewed_complete') is not True:
        issues.append('resource manifest is not reviewed complete')
    arches = manifest.get('architectures', [])
    if not arches or len(set(arches)) != len(arches) or not set(arches) <= {'arm64', 'x86_64'}:
        raise ValueError('explicit arm64/x86_64 architecture inventory required')
    floor = version(manifest['minimum_macos'])
    if floor != (14, 0, 0):
        raise ValueError('native inventory must support the macOS 14.0 floor')
    if not ROLES <= set(manifest.get('roles', {})):
        issues.append('missing owner disposition for engine/v8/extensions/agents/fonts/sparkle')
    for role, disposition in manifest.get('roles', {}).items():
        if not isinstance(disposition, dict) or disposition.get('kind') not in ('bundled', 'linked', 'not_required') or not disposition.get('evidence'):
            issues.append('missing resource disposition/evidence: ' + role)
    files, binaries = {}, {}
    for directory, dirs, names in os.walk(root, followlinks=False):
        for name in sorted(dirs + names):
            path = Path(directory) / name
            rel = path.relative_to(root).as_posix()
            if path.is_symlink():
                target = path.resolve(strict=True)
                if not inside(target, root):
                    issues.append('escaping symlink: ' + rel)
                files[rel] = {'symlink': os.readlink(path)}
            elif path.is_file():
                files[rel] = {'bytes': path.stat().st_size, 'sha256': digest(path)}
                with path.open('rb') as source:
                    magic = source.read(4)
                if magic in MACHO:
                    available = runner(['lipo', '-archs', str(path)]).split()
                    info = {}
                    if not set(arches) <= set(available):
                        issues.append('missing architecture: ' + rel)
                    # Check every delivered slice, even ones not requested.
                    for arch in available:
                        if arch not in {'arm64', 'x86_64'}:
                            issues.append('unsupported delivered architecture: ' + rel + ':' + arch)
                        info[arch] = commands(runner(['otool', '-arch', arch, '-l', str(path)]))
                        if version(info[arch]['minimum']) > floor:
                            issues.append('deployment target exceeds floor: ' + rel + ':' + arch)
                    binaries[rel] = info
    resources = manifest.get('resources', [])
    if not resources:
        issues.append('empty native resource inventory')
    declared = set()
    for resource in resources:
        rel = relative(resource['path']).as_posix()
        if rel in declared:
            raise ValueError('duplicate resource path')
        declared.add(rel)
        entry = files.get(rel)
        if not entry or 'sha256' not in entry or entry['sha256'] != resource['sha256']:
            issues.append('missing or changed declared resource: ' + rel)
        if resource.get('role') not in manifest.get('roles', {}):
            issues.append('resource has no owner role: ' + rel)
    for role, disposition in manifest.get('roles', {}).items():
        if disposition.get('kind') == 'bundled' and not any(r.get('role') == role for r in resources):
            issues.append('bundled role has no declared resource: ' + role)
    main = relative(manifest['executable']).as_posix()
    if main not in binaries:
        issues.append('missing Mach-O main executable: ' + main)
    # All delivered Mach-O files must be an explicit reviewed input, not an
    # accidental host-linked dependency hidden in an otherwise valid bundle.
    for rel in binaries:
        if rel not in declared:
            issues.append('undeclared Mach-O: ' + rel)

    def expand(value, loader, executable):
        if value == '@loader_path' or value.startswith('@loader_path/'):
            return (loader.parent / value[len('@loader_path'):].lstrip('/')).resolve()
        if value == '@executable_path' or value.startswith('@executable_path/'):
            return (executable.parent / value[len('@executable_path'):].lstrip('/')).resolve()
        return None

    # Traverse dependency graph with the actual executable and inherited rpaths.
    # A helper executable has its own @executable_path, unlike a loaded dylib.
    visited = set()
    def walk(rel, arch, executable, inherited, ancestors):
        if rel in ancestors:
            return
        data = binaries.get(rel, {}).get(arch)
        if data is None:
            issues.append('dependency has no required slice: ' + rel + ':' + arch)
            return
        loader = root / rel
        paths = []
        for value in data['rpaths']:
            path = expand(value, loader, executable)
            if path is None or not inside(path, root):
                issues.append('external or unsupported rpath: ' + rel + ':' + value)
            else:
                paths.append(path)
        paths += inherited
        visited.add((rel, arch))
        for dependency in data['dependencies']:
            if dependency.startswith(SYSTEM):
                continue
            if dependency.startswith('@rpath/'):
                candidates = [p / dependency[len('@rpath/'):] for p in paths]
            else:
                expanded = expand(dependency, loader, executable)
                candidates = [expanded] if expanded is not None else []
            target = next((p.resolve() for p in candidates if p.is_file() and inside(p.resolve(), root)), None)
            if target is None:
                issues.append('unresolved/external dependency: ' + rel + ':' + dependency)
                continue
            target_rel = target.relative_to(root).as_posix()
            if target_rel not in binaries:
                issues.append('dependency is not Mach-O: ' + target_rel)
                continue
            walk(target_rel, arch, executable, paths, ancestors | {rel})
    # Manifest declares every helper entry point explicitly. Resources cannot
    # invent fallback executables. Unreachable libraries still get audited below.
    entrypoints = [main] + [relative(x).as_posix() for x in manifest.get('helpers', [])]
    for entry in entrypoints:
        if entry not in binaries:
            issues.append('missing Mach-O entry point: ' + entry)
        for arch in binaries.get(entry, {}):
            walk(entry, arch, root / entry, [], set())
    for rel, slices in binaries.items():
        for arch in slices:
            if (rel, arch) not in visited:
                issues.append('Mach-O unreachable from declared executable/helpers: ' + rel + ':' + arch)
    return {'schema': 1, 'status': 'blocked' if issues else 'passed', 'issues': sorted(set(issues)),
            'files': files, 'macho': binaries, 'architectures': arches, 'minimum_macos': manifest['minimum_macos']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', required=True, type=Path)
    parser.add_argument('--requirements', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    try:
        result = audit(args.bundle, json.loads(args.requirements.read_text()))
        with args.output.open('x') as output:
            json.dump(result, output, indent=2, sort_keys=True)
            output.write('\n')
        print(result['status'] + ': ' + str(len(result['issues'])) + ' issues')
        return 0 if not result['issues'] else 1
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print('blocked: bundle audit could not complete: ' + str(error), file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
