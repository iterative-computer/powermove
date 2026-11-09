"""Linux contract tests: subprocess calls are recorded, never actually signed/submitted."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import shutil
import tempfile
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1] / "package.py"
spec = importlib.util.spec_from_file_location("packaging_commands", SOURCE)
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        # Every build input is synthetic or vendored in this owned test tree.
        # Canonicalizing also avoids macOS's root-owned /var and /tmp aliases.
        repo = self.root / "fixture-repo"
        native = repo / "apps/native"
        shutil.copytree(SOURCE.parent / "tests/fixtures/Assets.xcassets", native / "bundle/Assets.xcassets")
        for name in ("icon.icns", "document.icns"):
            (native / "bundle" / name).write_bytes(b"synthetic icon input")
        self.addCleanup(patch.stopall)
        patch.object(package, "ROOT", repo).start()
        patch.object(package, "NATIVE", native).start()
        self.binary = self.root / "native"
        self.binary.write_bytes(bytes.fromhex("cffaedfe") + b"synthetic native binary")
        self.binary.chmod(0o755)
        for helper in package.HELPERS:
            (self.root / helper).write_bytes(self.binary.read_bytes())
            (self.root / helper).chmod(0o755)
        self.app = self.root / "Powermove.app"
        self.config = self.root / "updates.json"
        self.config.write_bytes((package.HERE / "update-config.json").read_bytes())

    def bundle(self, channel="cutover", **kwargs):
        config = json.loads(self.config.read_text())
        config["channel"] = "cutover" if channel == "dev" else channel
        self.config.write_text(json.dumps(config))
        return package.bundle(self.binary, self.app, channel, "2.0.0", "12", self.config,
                              compile_assets=False, **kwargs)

    def test_bundle_layout_and_shipped_production_associations(self):
        self.bundle()
        info = plistlib.loads((self.app / "Contents/Info.plist").read_bytes())
        self.assertEqual(info["CFBundleIdentifier"], "com.zellzoi.powermove")
        self.assertEqual(info["CFBundleURLTypes"][0]["CFBundleURLSchemes"], ["powermove"])
        document = info["CFBundleDocumentTypes"][0]
        self.assertFalse(document["LSTypeIsPackage"])
        self.assertEqual(document["CFBundleTypeRole"], "Editor")
        self.assertEqual(document["LSItemContentTypes"], ["com.zellzoi.powermove.project"])
        self.assertEqual(info["UTExportedTypeDeclarations"][0]["UTTypeConformsTo"], ["public.data"])
        self.assertEqual(info["LSMinimumSystemVersion"], "14.0")
        self.assertEqual(info["NSAccentColorName"], "AccentColor")
        self.assertEqual((self.app / "Contents/MacOS/powermove-native").read_bytes(), self.binary.read_bytes())
        self.assertEqual((self.app / "Contents/PkgInfo").read_bytes(), b"APPL????")
        for name in ("icon.icns", "document.icns", "update-config.json"):
            self.assertTrue((self.app / "Contents/Resources" / name).is_file())
        self.assertFalse((self.app / "Contents/Resources/Assets.car").exists())
        self.assertNotIn("SUFeedURL", info)

    def test_helpers_and_agent_runtimes_land_where_discovery_reads_them(self):
        vendor = self.root / "vendor"
        (vendor / "bin").mkdir(parents=True)
        shutil.copy2(self.binary, vendor / "bin" / "codex")
        self.bundle(glsl_compiler=self.binary, claude_runtime=self.binary, codex_vendor=vendor)
        for path in ("MacOS/pm-glsl-compiler", "Resources/claude/bin/claude",
                     "Resources/codex/bin/codex"):
            self.assertTrue(os.access(self.app / "Contents" / path, os.X_OK), path)

    def test_accent_catalog_matches_the_existing_opus_owned_catalog_byte_for_byte(self):
        self.bundle()
        source = package.NATIVE / "bundle/Assets.xcassets"
        for path in source.rglob("*"):
            if path.is_file():
                copied = self.app / "Contents/Resources/Assets.xcassets" / path.relative_to(source)
                self.assertEqual(copied.read_bytes(), path.read_bytes())

    def test_beta_and_dev_do_not_take_production_protocol_or_document_ownership(self):
        for channel in ("beta", "dev"):
            info = package.info(channel, "2.0.0", "12")
            self.assertEqual(info["CFBundleIdentifier"], package.IDS[channel])
            self.assertNotIn("CFBundleDocumentTypes", info)
            self.assertNotIn("CFBundleURLTypes", info)

    def test_invalid_bundle_version_build_and_channel_fail(self):
        for channel, version, build in (("unknown", "2.0.0", "1"), ("cutover", "2.0.0-beta.1", "1"),
                                        ("cutover", "2.0.0", "-1"), ("cutover", "x", "1")):
            with self.assertRaises(ValueError):
                package.info(channel, version, build)

    def test_existing_bundle_and_linked_destination_are_preserved(self):
        self.bundle()
        before = (self.app / "Contents/Info.plist").read_bytes()
        with self.assertRaises(ValueError):
            self.bundle()
        self.assertEqual((self.app / "Contents/Info.plist").read_bytes(), before)
        link = self.root / "link"
        link.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(ValueError):
            package.new_destination(link / "new.app")

    def test_a_missing_worker_helper_fails_instead_of_packaging_a_broken_app(self):
        (self.root / "pm-store-helper").unlink()
        with self.assertRaisesRegex(ValueError, "pm-store-helper"):
            self.bundle()

    def test_non_native_or_linked_executable_is_rejected(self):
        self.binary.write_bytes(b"#!/bin/sh\nexec node main.js\n")
        with self.assertRaises(ValueError):
            self.bundle()
        self.binary.unlink()
        self.binary.symlink_to(package.HERE / "package.py")
        with self.assertRaises(ValueError):
            self.bundle()
        self.assertFalse(self.app.exists())

    def test_enabled_updates_require_pinned_keys_and_a_framework(self):
        value = json.loads(self.config.read_text())
        value.update(enabled=True, feedURL="https://updates.invalid/native/policy.json", publicKey=None)
        self.config.write_text(json.dumps(value))
        with self.assertRaises((ValueError, TypeError)):
            self.bundle()
        value["publicKey"] = "BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQU="
        self.config.write_text(json.dumps(value))
        with self.assertRaises(ValueError):
            self.bundle()
        self.assertFalse(self.app.exists())

    def test_sparkle_bootstrap_has_pinned_public_key_and_no_automatic_quit(self):
        key = "BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQU="
        info = package.info("cutover", "2.0.0", "1", "https://updates.invalid/native/appcast.xml", key)
        self.assertEqual(info["SUPublicEDKey"], key)
        self.assertFalse(info["SUEnableAutomaticChecks"])
        self.assertFalse(info["SUAutomaticallyUpdate"])
        with self.assertRaises(ValueError):
            package.info("cutover", "2.0.0", "1", "https://updates.invalid/native/appcast.xml")

    def test_config_channel_legacy_feeds_and_untrusted_urls_fail(self):
        original = json.loads(self.config.read_text())
        for url in ("http://updates.invalid/a", "https://u:p@updates.invalid/a", "https://updates.invalid/a?q=1",
                    "https://updates.invalid/a#x", "https://updates.invalid/latest-mac.yml"):
            value = copy.deepcopy(original)
            value.update(enabled=True, feedURL=url, publicKey="BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQU=")
            self.config.write_text(json.dumps(value))
            with self.assertRaises(ValueError):
                package.config(self.config, "cutover")
        self.config.write_text(json.dumps(original))
        with self.assertRaises(ValueError):
            package.config(self.config, "beta")

    def test_assets_compile_uses_existing_accent_and_minimum_os(self):
        with patch.object(package, "macos"), patch.object(package, "run") as run:
            package.bundle(self.binary, self.app, "cutover", "2.0.0", "12", self.config)
        command = [str(x) for x in run.call_args.args[0]]
        self.assertEqual(command[:2], ["xcrun", "actool"])
        self.assertIn("AccentColor", command)
        self.assertIn("14.0", command)

    def test_signing_requires_compiled_assets_and_runtime_identity(self):
        self.bundle()
        with patch.object(package, "macos"), patch.dict(os.environ, {}, clear=True), patch.object(package, "run") as run:
            with self.assertRaises(ValueError):
                package.sign(self.app)
            with self.assertRaises(ValueError):
                package.sign(self.app, "-")
            with self.assertRaises(ValueError):
                package.sign(self.app, "synthetic identity selector")
            run.assert_not_called()

    def test_signs_nested_code_inside_out_and_verifies_all_architectures(self):
        self.bundle()
        resources = self.app / "Contents/Resources"
        (resources / "Assets.car").write_bytes(b"synthetic catalog")
        helper = self.app / "Contents/Helpers/helper"
        helper.parent.mkdir()
        helper.write_bytes(self.binary.read_bytes())
        with patch.object(package, "macos"), patch.object(package, "run") as run:
            package.sign(self.app, "synthetic identity selector")
        calls = [[str(x) for x in c.args[0]] for c in run.call_args_list]
        self.assertEqual(calls[0][-1], str(helper))
        self.assertEqual(calls[-2][-1], str(self.app))
        self.assertIn("--entitlements", calls[-2])
        self.assertIn("--all-architectures", calls[-1])
        self.assertTrue(all("--deep" not in c for c in calls[:-1]))
        self.assertTrue(all("runtime" in c for c in calls[:-1]))
        self.assertTrue(all("requirements" not in arg for c in calls for arg in c))

    def test_signing_refuses_file_and_directory_symlinks_escaping_the_app(self):
        self.bundle()
        for directory in (False, True):
            link = self.app / "Contents/Resources/link"
            target = self.root if directory else self.binary
            link.symlink_to(target, target_is_directory=directory)
            with self.assertRaises(ValueError):
                package.nested_code(self.app)
            link.unlink()

    def test_dmg_contains_the_app_and_keeps_existing_staging_and_output(self):
        self.bundle()
        destination = self.root / "Powermove.dmg"
        with patch.object(package, "macos"), patch.object(package, "run") as run:
            package.dmg(self.app, destination)
            command = [str(x) for x in run.call_args.args[0]]
            staging = Path(command[command.index("-srcfolder") + 1])
            self.assertTrue((staging / "Powermove.app/Contents/Info.plist").exists())
            self.assertNotIn("-ov", command)
            with self.assertRaises(ValueError):
                package.dmg(self.app, destination)
        destination.write_bytes(b"retained existing DMG")
        with patch.object(package, "macos"), self.assertRaises(ValueError):
            package.dmg(self.app, destination)
        self.assertEqual(destination.read_bytes(), b"retained existing DMG")

    def test_transition_zip_is_full_keep_parent_archive_after_signature_verification(self):
        self.bundle()
        with patch.object(package, "macos"), patch.object(package, "run") as run:
            package.transition_zip(self.app, self.root / "Powermove.zip")
        calls = [[str(x) for x in c.args[0]] for c in run.call_args_list]
        self.assertEqual(calls[0][0], "codesign")
        self.assertEqual(calls[1][:4], ["ditto", "-c", "-k", "--sequesterRsrc"])
        self.assertIn("--keepParent", calls[1])

    def test_notarization_uses_existing_keychain_profile_only_and_is_explicit(self):
        artifact = self.root / "Powermove.dmg"
        artifact.write_bytes(b"synthetic DMG")
        with patch.object(package, "macos"), patch.dict(os.environ, {}, clear=True), patch.object(package, "run") as run:
            with self.assertRaises(ValueError):
                package.notarize(artifact)
            run.assert_not_called()
            package.notarize(artifact, "synthetic-keychain-profile")
        calls = [[str(x) for x in c.args[0]] for c in run.call_args_list]
        self.assertEqual(calls[0][:3], ["xcrun", "notarytool", "submit"])
        self.assertIn("--keychain-profile", calls[0])
        self.assertEqual(calls[1][:3], ["xcrun", "stapler", "staple"])
        self.assertEqual(calls[2][:3], ["xcrun", "stapler", "validate"])
        self.assertEqual(calls[3][0], "spctl")
        self.assertTrue(all("--password" not in c and "--apple-id" not in c for c in calls))

    def test_notarize_app_archives_for_submission_then_staples_app(self):
        self.bundle()
        with patch.object(package, "macos"), patch.object(package, "run") as run:
            package.notarize(self.app, "synthetic-keychain-profile")
        calls = [[str(x) for x in c.args[0]] for c in run.call_args_list]
        self.assertEqual(calls[1][0], "ditto")
        self.assertTrue(calls[2][3].endswith(".notary.zip"))
        self.assertEqual(calls[3][-1], str(self.app))
        self.assertEqual(calls[-1][1:5], ["--assess", "--type", "execute", str(self.app)])

    def test_sparkle_framework_version_pin_is_required_before_copying(self):
        framework = self.root / "Sparkle.framework"
        (framework / "Resources").mkdir(parents=True)
        path = framework / "Resources/Info.plist"
        path.write_bytes(plistlib.dumps({"CFBundleShortVersionString": "0.0.0"}))
        with self.assertRaises(ValueError):
            self.bundle(sparkle_framework=framework)
        self.assertFalse(self.app.exists())
        pin = json.loads((package.HERE / "sparkle-pin.json").read_text())
        path.write_bytes(plistlib.dumps({"CFBundleShortVersionString": pin["version"]}))
        self.bundle(sparkle_framework=framework)
        self.assertTrue((self.app / "Contents/Frameworks/Sparkle.framework/Resources/Info.plist").is_file())

    def test_upstream_sparkle_archive_digest_is_checked_without_extracting(self):
        archive = self.root / "sparkle.tar.xz"
        archive.write_bytes(b"synthetic archive")
        with self.assertRaises(ValueError):
            package.verify_sparkle_archive(archive)
        pin = {"sha256": package.hashlib.sha256(archive.read_bytes()).hexdigest()}
        original = Path.read_text
        with patch.object(Path, "read_text", lambda path, *a, **kw: json.dumps(pin) if path.name == "sparkle-pin.json" else original(path, *a, **kw)):
            package.verify_sparkle_archive(archive)
        self.assertEqual(archive.read_bytes(), b"synthetic archive")

    def test_entitlements_are_minimal_and_have_no_broad_keychain_or_memory_bypass(self):
        app = plistlib.loads((package.HERE / "entitlements/app.plist").read_bytes())
        helper = plistlib.loads((package.HERE / "entitlements/helper.plist").read_bytes())
        self.assertEqual(app, {"com.apple.security.cs.allow-jit": True})
        self.assertEqual(helper, {})


class AdhocProofSafetyTests(unittest.TestCase):
    def test_adhoc_cannot_sign_production_or_owner_bundle(self):
        with tempfile.TemporaryDirectory() as temp:
            app = Path(temp).resolve() / "Production.app"
            (app / "Contents/Resources").mkdir(parents=True)
            (app / "Contents/Info.plist").write_bytes(plistlib.dumps({"CFBundleIdentifier": "com.zellzoi.powermove"}))
            (app / "Contents/Resources/Assets.car").write_bytes(b"synthetic")
            with patch.object(package, "macos"), patch.object(package, "run") as calls:
                with self.assertRaises(ValueError):
                    package.sign(app, adhoc_proof=True)
                calls.assert_not_called()

    def test_adhoc_proof_never_uses_identity_or_timestamp_service(self):
        with tempfile.TemporaryDirectory() as temp:
            app = Path(temp).resolve() / "Synthetic.app"
            (app / "Contents/Resources").mkdir(parents=True)
            (app / "Contents/Info.plist").write_bytes(plistlib.dumps({"CFBundleIdentifier": "com.powermove.signature-proof"}))
            (app / "Contents/Resources/Assets.car").write_bytes(b"synthetic")
            with patch.object(package, "macos"), patch.object(package, "run") as calls:
                package.sign(app, identity="must-never-use", adhoc_proof=True)
                sign = calls.call_args_list[0].args[0]
                self.assertIn("--timestamp=none", sign)
                self.assertEqual(sign[sign.index("--sign") + 1], "-")


if __name__ == "__main__":
    unittest.main()
