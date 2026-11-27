#!/usr/bin/env python3
"""Read-only signed candidate proof. Never signs, submits, installs or publishes.

Run on a Mac against an existing candidate and an owner-reviewed per-file
inventory. Reports are private local evidence, bound to the full bundle hashes.
Python 3.9+. Exit 0 is this static proof only, never beta/cutover approval.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import plistlib
import re
import subprocess
import sys

SPEC = importlib.util.spec_from_file_location('native_bundle_audit', Path(__file__).with_name('audit-native-bundle.py'))
AUDIT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(AUDIT)


def run(args):
    result = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
    return (result.stdout + result.stderr).decode('utf-8')


def entitlement_plist(text):
    start = text.find('<?xml')
    end = text.find('</plist>', start)
    if start < 0 or end < 0:
        raise ValueError('codesign did not return XML entitlements')
    return plistlib.loads(text[start:end + len('</plist>')].encode('utf-8'))


def prove(bundle, requirements, runner=run, auditor=AUDIT.audit):
    root = Path(bundle).resolve(strict=True)
    inventory = auditor(root, requirements, runner)
    issues = list(inventory['issues'])
    observations = {}
    signatures = requirements.get('signatures', {})
    if set(signatures) != set(inventory['macho']):
        issues.append('exact signature requirements missing or extra for Mach-O inventory')
    for relative, slices in inventory['macho'].items():
        expected = signatures.get(relative)
        if not isinstance(expected, dict):
            continue
        team = expected.get('team_id')
        identifier = expected.get('identifier')
        requirement = expected.get('requirement')
        entitlements = expected.get('entitlements')
        if (not isinstance(team, str) or not re.fullmatch('[A-Z0-9]{10}', team)
                or not isinstance(identifier, str) or not identifier
                or not isinstance(requirement, str) or not requirement.strip()
                or not isinstance(entitlements, dict)):
            raise ValueError('complete reviewed signature policy required: ' + relative)
        path = str(root / relative)
        observations[relative] = {}
        for arch in slices:
            try:
                before_issues = len(issues)
                runner(['codesign', '--verify', '--strict', '--all-architectures', '-R=' + requirement, path])
                details = runner(['codesign', '--display', '--verbose=4', '--arch', arch, path])
                if (('TeamIdentifier=' + team) not in details.splitlines()
                        or ('Identifier=' + identifier) not in details.splitlines()
                        or not re.search(r'^CodeDirectory .*\bflags=.*\bruntime\b', details, re.MULTILINE)
                        or 'Signature=adhoc' in details):
                    issues.append('identity/hardened-runtime mismatch: ' + relative + ':' + arch)
                # codesign emits no plist for an empty entitlement set. Explicit
                # --entitlements output must still be inspected for unexpected keys.
                encoded = runner(['codesign', '--display', '--arch', arch, '--entitlements', ':-', path])
                actual = entitlement_plist(encoded) if '<?xml' in encoded else {}
                if actual != entitlements:
                    issues.append('entitlements differ from reviewed policy: ' + relative + ':' + arch)
                observations[relative][arch] = {'verified': len(issues) == before_issues, 'entitlements': actual}
            except (subprocess.CalledProcessError, OSError, ValueError) as error:
                issues.append('signature check failed: ' + relative + ':' + arch + ':' + type(error).__name__)
                observations[relative][arch] = {'verified': False}
    for name, command in (
            ('deep_signature', ['codesign', '--verify', '--deep', '--strict', '--all-architectures', str(root)]),
            ('gatekeeper', ['spctl', '--assess', '--type', 'execute', '--verbose=4', str(root)]),
            ('stapled_ticket', ['xcrun', 'stapler', 'validate', str(root)])):
        try:
            runner(command)
            observations[name] = True
        except (subprocess.CalledProcessError, OSError):
            observations[name] = False
            issues.append(name + ' failed')
    after = auditor(root, requirements, runner)
    if inventory['files'] != after['files']:
        issues.append('bundle changed during signed proof')
    issues.extend(after['issues'])
    return {'schema': 1, 'status': 'blocked' if issues else 'passed',
            'scope': 'static signed candidate; no runtime, installation or release approval',
            'issues': sorted(set(issues)), 'inventory': inventory, 'signatures': observations}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--requirements', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        if sys.platform != 'darwin':
            raise ValueError('signed-build proof requires macOS; Linux runs only command-double tests')
        # Reserve a new report before work, retaining an explicit failed attempt.
        with args.output.open('x') as output:
            try:
                result = prove(args.bundle, json.loads(args.requirements.read_text()))
            except Exception as error:
                json.dump({'schema': 1, 'status': 'blocked', 'issues': [type(error).__name__]}, output)
                output.write('\n')
                raise
            json.dump(result, output, indent=2, sort_keys=True)
            output.write('\n')
        print(result['status'] + ': ' + str(len(result['issues'])) + ' issues')
        return 0 if not result['issues'] else 1
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print('blocked: signed proof could not complete: ' + type(error).__name__, file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
