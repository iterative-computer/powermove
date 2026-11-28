"""Linux contract tests: subprocess calls are recorded, never actually signed/submitted."""
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import shutil
import tempfile
import unittest
import unittest.mock
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
        self.ffmpeg_dir = self.root / "ffmpeg-out"
        (self.ffmpeg_dir / "bin").mkdir(parents=True)
        for tool in ("ffmpeg", "ffprobe"):
            shutil.copy2(self.binary, self.ffmpeg_dir / "bin" / tool)
        (self.ffmpeg_dir / "lib").mkdir()
        shutil.copy2(self.binary, self.ffmpeg_dir / "lib/libavformat.63.1.102.dylib")
        (self.ffmpeg_dir / "lib/libavformat.63.dylib").symlink_to("libavformat.63.1.102.dylib")
        (self.ffmpeg_dir / "lib/libavformat.dylib").symlink_to("libavformat.63.1.102.dylib")
        patch.dict(os.environ, {"PM_FFMPEG_DIR": str(self.ffmpeg_dir)}).start()
        self.app = self.root / "Powermove.app"

    def test_media_tools_are_bundled_and_relocated(self):
        with patch.object(package, "macos"), patch.object(package, "run") as run:
            package.bundle(self.binary, self.app, "cutover", "2.0.0", "12")
        for tool in ("ffmpeg", "ffprobe"):
            target = self.app / "Contents/MacOS" / tool
            self.assertEqual(target.read_bytes(), self.binary.read_bytes())
            self.assertTrue(os.access(target, os.X_OK))
            run.assert_any_call(["install_name_tool", "-rpath", "@executable_path/../lib",
                "@executable_path/../Frameworks", target])
            self.assertIn(target, package.nested_code(self.app))

    def test_ffmpeg_libraries_travel_under_their_install_names(self):
        with patch.object(package, "macos"), patch.object(package, "run") as run, \
                patch.object(package, "rpaths", return_value=["/build/ffmpeg/out/lib", "@executable_path/../Frameworks"]):
            package.bundle(self.binary, self.app, "cutover", "2.0.0", "12")
        frameworks = self.app / "Contents/Frameworks"
        self.assertEqual(sorted(p.name for p in frameworks.iterdir()), ["libavformat.63.dylib"])
        self.assertFalse((frameworks / "libavformat.63.dylib").is_symlink())
        self.assertIn(frameworks / "libavformat.63.dylib", package.nested_code(self.app))
        run.assert_any_call(["install_name_tool", "-delete_rpath", "/build/ffmpeg/out/lib",
                             self.app / "Contents/MacOS/Powermove"])
        self.assertNotIn(["install_name_tool", "-delete_rpath", "@executable_path/../Frameworks",
                          self.app / "Contents/MacOS/Powermove"], [c.args[0] for c in run.call_args_list])

    def test_missing_media_tool_refuses_bundle(self):
        (self.ffmpeg_dir / "bin/ffprobe").unlink()
        with self.assertRaises((ValueError, FileNotFoundError)):
            self.bundle()

    def bundle(self, channel="cutover", **kwargs):
        return package.bundle(self.binary, self.app, channel, "2.0.0", "12",
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
        self.assertEqual((self.app / "Contents/MacOS/Powermove").read_bytes(), self.binary.read_bytes())
        self.assertEqual((self.app / "Contents/PkgInfo").read_bytes(), b"APPL????")
        for name in ("icon.icns", "document.icns"):
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

    def test_only_dev_bundles_take_their_own_lane_id(self):
        self.assertEqual(package.info("dev", "2.0.0", "1", bundle_id="com.powermove.native.dev.w122")
                         ["CFBundleIdentifier"], "com.powermove.native.dev.w122")
        for channel, value in (("cutover", "com.powermove.native.dev.w122"), ("dev", "com.zellzoi.powermove"),
                               ("dev", "com.powermove.native.dev")):
            with self.assertRaises(ValueError):
                package.info(channel, "2.0.0", "1", bundle_id=value)

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

    def test_sparkle_needs_framework_feed_and_key_together(self):
        key = "BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQU="
        with self.assertRaises(ValueError):
            self.bundle(sparkle_feed="https://updates.invalid/appcast.xml", sparkle_key=key)
        self.assertFalse(self.app.exists())
        with self.assertRaises(ValueError):
            package.info("cutover", "2.0.0", "1", "https://updates.invalid/native/appcast.xml")

    def test_sparkle_checks_daily_asks_before_installing_and_requires_signed_feed(self):
        key = "BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQU="
        info = package.info("cutover", "2.0.0", "1", "https://updates.invalid/native/appcast.xml", key)
        self.assertEqual(info["SUPublicEDKey"], key)
        self.assertTrue(info["SUEnableAutomaticChecks"])
        self.assertFalse(info["SUAutomaticallyUpdate"])
        self.assertTrue(info["SUVerifyUpdateBeforeExtraction"])
        self.assertTrue(info["SURequireSignedFeed"])

    def test_shipping_feeds_are_https_and_dev_feeds_are_loopback_only(self):
        key = "BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQU="
        for url in ("http://updates.invalid/a", "https://u:p@updates.invalid/a", "https://updates.invalid/a?q=1",
                    "https://updates.invalid/a#x", "https://updates.invalid/latest-mac.yml",
                    "http://127.0.0.1:8765/appcast.xml"):
            with self.assertRaises(ValueError):
                package.info("cutover", "2.0.0", "1", url, key)
        # A dev or lane build never names the real feed.
        for url in ("https://github.com/iterative-computer/powermove/releases/latest/download/appcast.xml",
                    "http://localhost.invalid/appcast.xml", "http://u:p@127.0.0.1/appcast.xml"):
            with self.assertRaises(ValueError):
                package.info("dev", "2.0.0", "1", url, key)
        for url in ("http://127.0.0.1:8765/appcast.xml", "http://[::1]:8765/appcast.xml"):
            self.assertEqual(package.info("dev", "2.0.0", "1", url, key)["SUFeedURL"], url)

    def test_release_resources_and_transcribe_runtime_are_bundled(self):
        runtime = self.root / "transcribe"
        runtime.mkdir()
        (runtime / "contract.json").write_text("{}")
        (runtime / "libtranscribe.dylib").write_bytes(self.binary.read_bytes())
        for name, source in package.RESOURCES.items():
            path = package.NATIVE / source
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"synthetic " + name.encode())
        self.bundle(transcribe_runtime=runtime)
        resources = self.app / "Contents/Resources"
        self.assertTrue((resources / "transcribe/contract.json").is_file())
        self.assertIn(resources / "transcribe/libtranscribe.dylib", package.nested_code(self.app))
        for name in package.RESOURCES:
            self.assertTrue((resources / name).is_file(), name)

    def test_assets_compile_uses_existing_accent_and_minimum_os(self):
        with patch.object(package, "macos"), patch.object(package, "run") as run:
            package.bundle(self.binary, self.app, "cutover", "2.0.0", "12")
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
        nested = [c[-1] for c in calls[:-2]]
        self.assertIn(str(helper), nested)  # inside out: nested code before the app
        self.assertIn(str(self.app / "Contents/Frameworks/libavformat.63.dylib"), nested)
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

    def test_dmg_is_create_dmgs_window_signed_and_keeps_existing_output(self):
        self.bundle()
        destination = self.root / "Powermove.dmg"

        def fake_run(command, *_, **__):
            command = [str(x) for x in command]
            if any(c.startswith("create-dmg@") for c in command):
                (Path(command[-1]) / "Powermove.dmg").write_bytes(b"dmg")

        with patch.object(package, "macos"), patch.object(package, "run", side_effect=fake_run) as run, \
                patch.dict(package.os.environ, {"PM_CODESIGN_IDENTITY": "Developer ID Application: Test (TEAM)"}):
            package.dmg(self.app, destination)
            calls = [[str(x) for x in c.args[0]] for c in run.call_args_list]
            create = next(c for c in calls if any(x.startswith("create-dmg@") for x in c))
            self.assertIn("--no-code-sign", create)
            self.assertEqual(create[-2], str(self.app.resolve()))
            self.assertEqual(calls[-1][:2], ["codesign", "--force"])
            self.assertEqual(calls[-1][-1], str(destination))
            self.assertEqual(destination.read_bytes(), b"dmg")
            with self.assertRaises(ValueError):
                package.dmg(self.app, destination)
        other = self.root / "Other.dmg"
        other.write_bytes(b"retained existing DMG")
        with patch.object(package, "macos"), self.assertRaises(ValueError):
            package.dmg(self.app, other)
        self.assertEqual(other.read_bytes(), b"retained existing DMG")

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
        with patch.object(package, "macos"), patch.dict(os.environ, {"PM_NOTARY_KEY": str(artifact),
                "PM_NOTARY_KEY_ID": "KEYID", "PM_NOTARY_ISSUER": "issuer"}, clear=True), \
                patch.object(package, "run") as api:
            package.notarize(artifact)
        submit = [str(x) for x in api.call_args_list[0].args[0]]
        self.assertEqual(submit[submit.index("--key") + 1], str(artifact))
        self.assertNotIn("--keychain-profile", submit)
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
        sparkle = dict(sparkle_framework=framework, sparkle_feed="https://updates.invalid/appcast.xml",
                       sparkle_key="BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQU=")
        with self.assertRaises(ValueError):
            self.bundle(**sparkle)
        self.assertFalse(self.app.exists())
        pin = json.loads((package.HERE / "sparkle-pin.json").read_text())
        path.write_bytes(plistlib.dumps({"CFBundleShortVersionString": pin["version"]}))
        self.bundle(**sparkle)
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
                self.assertNotIn("runtime", sign)
                self.assertEqual(sign[sign.index("--sign") + 1], "-")


class ReleaseFeedTests(unittest.TestCase):
    """The feeds V1's electron-updater and V2's Sparkle read, from one ZIP."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.zip = self.root / "Powermove-2.0.0-mac-arm64.zip"
        self.zip.write_bytes(b"synthetic signed app archive")
        self.signed = []

    def feeds(self, **kwargs):
        def sign(tool, key, path, *extra):
            self.signed.append((Path(path).name, extra))
            return "c2ln" if extra == ("-p",) else ""
        with patch.object(package, "sparkle_sign", sign), patch.object(package, "verify_ed25519") as verify:
            out = package.feeds(self.zip, self.root / "feeds", "2.0.0", "2.0.0", "v2.0.0",
                                "iterative-computer/powermove", "sign_update", "key",
                                "BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQU=",
                                date=package.datetime(2026, 11, 27, 9, 30, tzinfo=package.timezone.utc), **kwargs)
        verify.assert_called_once()
        return out

    def test_electron_feed_matches_what_electron_updater_reads(self):
        out = self.feeds()
        digest = package.base64.b64encode(package.hashlib.sha512(self.zip.read_bytes()).digest()).decode()
        latest = (out / "latest-mac.yml").read_text()
        self.assertEqual(latest, (out / "beta-mac.yml").read_text())
        self.assertEqual(latest, f"""version: 2.0.0
files:
  - url: Powermove-2.0.0-mac-arm64.zip
    sha512: {digest}
    size: {self.zip.stat().st_size}
minimumSystemVersion: 23.0.0
path: Powermove-2.0.0-mac-arm64.zip
sha512: {digest}
releaseDate: '2026-11-27T09:30:00.000Z'
""")
        # electron-updater keeps arm64 files for Apple silicon V1 (MacUpdater.js).
        self.assertIn("arm64", self.zip.name)

    def test_sparkle_appcast_is_signed_and_points_at_the_tagged_archive(self):
        out = self.feeds()
        appcast = (out / "appcast.xml").read_text()
        self.assertIn('url="https://github.com/iterative-computer/powermove/releases/download/v2.0.0/'
                      'Powermove-2.0.0-mac-arm64.zip"', appcast)
        self.assertIn('sparkle:edSignature="c2ln"', appcast)
        self.assertIn(f'length="{self.zip.stat().st_size}"', appcast)
        self.assertIn("<sparkle:version>2.0.0</sparkle:version>", appcast)
        self.assertIn("<sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>", appcast)
        self.assertEqual(self.signed, [(self.zip.name, ("-p",)), ("appcast.xml", ())])
        import xml.etree.ElementTree as ET
        ET.fromstring(appcast)

    def test_feeds_refuse_wrong_names_tags_and_existing_outputs(self):
        with self.assertRaises(ValueError):
            package.feeds(self.zip, self.root / "a", "2.0.1", "2.0.1", "v2.0.1", "o/r", "s", "k", "x")
        with self.assertRaises(ValueError):
            package.feeds(self.zip, self.root / "a", "2.0.0", "2.0.0", "2.0.0", "o/r", "s", "k", "x")
        self.feeds()
        with self.assertRaises(ValueError):
            self.feeds()

    def test_v1_requirement_is_the_shipped_production_identity(self):
        requirement = package.v1_requirement()
        self.assertTrue(requirement.startswith('identifier "com.zellzoi.powermove" and anchor apple generic'))
        self.assertIn('certificate leaf[subject.OU] = "8JTS5XWJJN"', requirement)
        self.assertEqual(package.IDS["cutover"], "com.zellzoi.powermove")
        self.assertEqual(package.info("cutover", "2.0.0", "2.0.0")["CFBundleIdentifier"], "com.zellzoi.powermove")

    def test_continuity_checks_v1_requirement_identifier_and_arm64(self):
        app = self.root / "Powermove.app"
        (app / "Contents/MacOS").mkdir(parents=True)
        info = {"CFBundleIdentifier": "com.zellzoi.powermove", "CFBundleExecutable": "Powermove"}
        (app / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
        lipo = unittest.mock.Mock(stdout="arm64\n")
        with patch.object(package, "macos"), patch.object(package.subprocess, "run", return_value=lipo):
            calls = []
            package.continuity(app, run=calls.append)
            self.assertEqual(calls[0][-3:], ["-R", "=" + package.v1_requirement(), app])
            lipo.stdout = "x86_64\n"
            with self.assertRaises(ValueError):
                package.continuity(app, run=calls.append)
            info["CFBundleIdentifier"] = "com.zellzoi.powermove.native-beta"
            (app / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
            with self.assertRaises(ValueError):
                package.continuity(app, run=calls.append)

    def test_release_script_archive_and_feeds_do_not_require_cargo_target_dir(self):
        script = package.HERE / "release.sh"
        env = {k: v for k, v in os.environ.items() if k != "CARGO_TARGET_DIR"}
        env.pop("PM_RELEASE_VERSION", None)
        for command in ("archive", "feeds"):
            result = package.subprocess.run(["bash", str(script), command], env=env, capture_output=True, text=True)
            self.assertNotIn("CARGO_TARGET_DIR", result.stderr)
            self.assertIn("PM_RELEASE_VERSION", result.stderr)


if __name__ == "__main__":
    unittest.main()
