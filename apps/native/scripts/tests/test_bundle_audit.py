"""Synthetic Mach-O command doubles; no Apple tool or real app is required."""
import importlib.util
import pathlib
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('bundle_audit', pathlib.Path(__file__).resolve().parents[1] / 'audit-native-bundle.py')
audit = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(audit)


def commands(dependencies=(), rpaths=(), floor='14.0', platform='1'):
    items = ['cmd LC_BUILD_VERSION\n platform ' + platform + '\n minos ' + floor + '\n sdk 15.0\n']
    items += ['cmd LC_RPATH\n path ' + path + ' (offset 12)\n' for path in rpaths]
    items += ['cmd LC_LOAD_DYLIB\n name ' + path + ' (offset 24)\n' for path in dependencies]
    return 'synthetic:\n' + ''.join('Load command ' + str(i) + '\n ' + item for i, item in enumerate(items))


class BundleAudit(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name).resolve() / 'Synthetic.app'
        self.root.mkdir()
        self.main = 'Contents/MacOS/native'
        self.lib = 'Contents/Frameworks/engine.dylib'
        self.data = 'Contents/Resources/v8.snapshot'
        for rel in (self.main, self.lib, self.data):
            path = self.root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'\xcf\xfa\xed\xfe' + b'synthetic' if rel != self.data else b'synthetic snapshot')
        self.loads = {self.main: commands(['@rpath/engine.dylib', '/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit'], ['@executable_path/../Frameworks']), self.lib: commands()}
        self.arches = 'arm64 x86_64'
        self.manifest = {'schema': 1, 'reviewed_complete': True, 'architectures': ['arm64', 'x86_64'], 'minimum_macos': '14.0', 'executable': self.main,
                         'roles': {role: {'kind': 'linked', 'evidence': 'synthetic fixture'} for role in audit.ROLES},
                         'resources': [{'path': rel, 'role': 'engine', 'sha256': audit.digest(self.root / rel)} for rel in (self.main, self.lib, self.data)]}

    def runner(self, args):
        if args[0] == 'lipo':
            return self.arches
        return self.loads[pathlib.Path(args[-1]).relative_to(self.root).as_posix()]

    def result(self):
        return audit.audit(self.root, self.manifest, self.runner)

    def test_complete_relocated_dependency_graph_passes(self):
        self.assertEqual(self.result()['issues'], [])

    def test_missing_resource_and_changed_hash_stay_visible(self):
        (self.root / self.data).write_bytes(b'changed')
        self.assertTrue(any('changed declared resource' in issue for issue in self.result()['issues']))
        (self.root / self.data).unlink()
        self.assertEqual(self.result()['status'], 'blocked')

    def test_host_absolute_load_and_rpath_are_rejected(self):
        self.loads[self.main] = commands(['/opt/local/lib/libengine.dylib'], ['/opt/local/lib'])
        self.assertEqual(self.result()['status'], 'blocked')
        self.assertTrue(any('external or unsupported rpath' in issue for issue in self.result()['issues']))
        self.assertTrue(any('unresolved/external dependency' in issue for issue in self.result()['issues']))

    def test_each_architecture_and_deployment_target_checked(self):
        self.arches = 'arm64'
        self.assertTrue(any('missing architecture' in issue for issue in self.result()['issues']))
        self.arches = 'arm64 x86_64'
        self.loads[self.lib] = commands(floor='15.0')
        self.assertTrue(any('deployment target exceeds floor' in issue for issue in self.result()['issues']))

    def test_escaping_framework_link_never_counts_as_dependency(self):
        (self.root / self.lib).unlink()
        outside = self.root.parent / 'foreign.dylib'
        outside.write_bytes(b'synthetic')
        (self.root / self.lib).symlink_to(outside)
        self.assertTrue(any('escaping symlink' in issue for issue in self.result()['issues']))
        self.assertTrue(any('unresolved/external dependency' in issue for issue in self.result()['issues']))

    def test_contained_framework_links_work(self):
        actual = self.root / 'Contents/Frameworks/Versions/A/engine.dylib'
        actual.parent.mkdir(parents=True)
        (self.root / self.lib).rename(actual)
        (self.root / self.lib).symlink_to('Versions/A/engine.dylib')
        self.loads[actual.relative_to(self.root).as_posix()] = commands()
        resource = self.manifest['resources'][1]
        resource['path'] = actual.relative_to(self.root).as_posix()
        self.assertEqual(self.result()['issues'], [])

    def test_inherited_rpath_load_and_loader_path(self):
        leaf = self.root / 'Contents/Frameworks/leaf.dylib'
        leaf.write_bytes(b'\xcf\xfa\xed\xfe')
        self.loads[self.lib] = commands(['@rpath/leaf.dylib'])
        self.loads[leaf.relative_to(self.root).as_posix()] = commands()
        self.manifest['resources'].append({'path': leaf.relative_to(self.root).as_posix(), 'role': 'engine', 'sha256': audit.digest(leaf)})
        self.assertEqual(self.result()['issues'], [])
        self.loads[self.lib] = commands(['@loader_path/leaf.dylib'])
        self.assertEqual(self.result()['issues'], [])

    def test_missing_slice_in_loaded_library_and_undeclared_binary(self):
        original = self.runner
        def runner(args):
            if args[0] == 'lipo' and args[-1].endswith('engine.dylib'):
                return 'arm64'
            return original(args)
        result = audit.audit(self.root, self.manifest, runner)
        self.assertTrue(any('dependency has no required slice' in issue for issue in result['issues']))
        self.manifest['resources'].pop(1)
        self.assertTrue(any('undeclared Mach-O' in issue for issue in self.result()['issues']))

    def test_unreviewed_or_empty_inventory_never_passes(self):
        self.manifest['reviewed_complete'] = False
        self.manifest['resources'] = []
        self.manifest['roles'].pop('v8')
        self.assertEqual(self.result()['status'], 'blocked')
        self.assertTrue(any('missing owner disposition' in issue for issue in self.result()['issues']))


    def test_bundled_role_requires_explicit_resource(self):
        self.manifest['roles']['fonts']['kind'] = 'bundled'
        self.assertTrue(any('bundled role has no declared resource: fonts' in issue for issue in self.result()['issues']))

    def test_truthy_review_flag_is_not_review(self):
        self.manifest['reviewed_complete'] = 'false'
        self.assertEqual(self.result()['status'], 'blocked')

    def test_missing_deployment_or_nonmac_platform_is_error(self):
        with self.assertRaises(ValueError):
            audit.commands('Load command 0\n cmd LC_RPATH\n path @loader_path (offset 12)\n')
        with self.assertRaises(ValueError):
            audit.commands(commands(platform='2'))

    def test_unsafe_and_duplicate_resource_paths_are_errors(self):
        self.manifest['resources'][0]['path'] = '../foreign'
        with self.assertRaises(ValueError):
            self.result()
        self.manifest['resources'][0]['path'] = self.lib
        with self.assertRaises(ValueError):
            self.result()

    def test_helper_uses_own_executable_anchor(self):
        helper = 'Contents/Helpers/native-helper'
        path = self.root / helper
        path.parent.mkdir(parents=True)
        path.write_bytes(b'\xcf\xfa\xed\xfe')
        self.loads[helper] = commands(['@executable_path/../Frameworks/engine.dylib'])
        self.manifest['helpers'] = [helper]
        self.manifest['resources'].append({'path': helper, 'role': 'agents', 'sha256': audit.digest(path)})
        self.assertEqual(self.result()['issues'], [])


if __name__ == '__main__':
    unittest.main()
