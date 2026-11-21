#!/usr/bin/env python3
"""Sign the dry-run app and DMG and record only checked public proof fields."""
import base64
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import secrets
import subprocess
import tempfile
import time
from uuid import UUID
from xml.etree import ElementTree as XML

from package import HERE, NATIVE, info, verify_release_versions

IDENTITY = "Developer ID Application"
RELEASE_ROOT = "https://cloud.trypowermove.com/native"
DOMAIN = b"Powermove Native update policy v1\0"
SPARKLE = "http://www.andymatuschak.org/xml-namespaces/sparkle"
SECRET_ENV = {
    "APPLE_CERTIFICATE_P12", "APPLE_CERTIFICATE_PASSWORD", "APPLE_API_KEY_P8",
    "APPLE_API_KEY_ID", "APPLE_API_ISSUER_ID", "SPARKLE_ED_PRIVATE_KEY",
    "POWERMOVE_UPDATE_POLICY_KEY",
}


def capture(args, *, env=None, data=None):
    child_env = {key: value for key, value in (os.environ if env is None else env).items()
                 if key not in SECRET_ENV}
    return subprocess.run(args, check=False, capture_output=True, env=child_env, input=data)


def checked(label, args, *, env=None, data=None):
    result = capture(args, env=env, data=data)
    if result.returncode:
        raise ValueError(f"{label} failed (exit {result.returncode})")
    return result.stdout, result.stderr


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def verify_runtime_versions(app, scratch):
    pins = json.loads((NATIVE / "crates/agents/fixtures/runtime-pins.json").read_text())
    for name, expected in (("codex", "codex-cli " + pins["codex"]),
                           ("claude", pins["claude"] + " (Claude Code)")):
        binary = app / "Contents/Resources" / name / "bin" / name
        out, _ = checked(name + " version", [binary, "--version"], env={"HOME": str(scratch), "LC_ALL": "C"})
        if out.decode().strip() != expected:
            raise ValueError(f"Bundled {name} version differs from its pin")
    return {name: pins[name] for name in ("codex", "claude")}


NOTARY_STATES = ("Accepted", "Invalid", "Rejected", "In Progress")
NOTARY_ISSUES = {
    "not signed with a valid Developer ID": "invalid Developer ID signature",
    "valid Developer ID certificate": "invalid Developer ID signature",
    "secure timestamp": "missing secure timestamp",
    "hardened runtime": "missing hardened runtime",
    "disallowed entitlement": "disallowed entitlement",
}


def notary_credentials(work):
    private_key, key_id, issuer = (
        os.environ.pop(name, "").strip()
        for name in ("APPLE_API_KEY_P8", "APPLE_API_KEY_ID", "APPLE_API_ISSUER_ID")
    )
    if not private_key or not key_id or not issuer:
        raise ValueError("Apple notarization team credentials are incomplete")
    key = work / "notary.p8"
    key.write_text(private_key + "\n")
    return ["--key", key, "--key-id", key_id, "--issuer", issuer]


def notary_result_json(result):
    try:
        value = json.loads(result.stdout)
        return value if isinstance(value, dict) else {}
    except (UnicodeError, ValueError):
        return {}


def notary_failure(result):
    # Transport errors can contain signed upload URLs or the API key path.
    # Return only fixed labels and numeric protocol codes.
    raw = result.stderr + b"\n" + result.stdout
    http = re.search(rb"(?i)(?:HTTP(?: status(?: code)?| error)?|statusCode)[: ]+([45][0-9]{2})\b", raw)
    if http:
        return f"HTTP {http[1].decode()}"
    for label, marker in (("invalid API credentials", b"invalid credentials"),
                          ("Apple rejected the API key", b"could not validate the key"),
                          ("upload timed out", b"timed out"),
                          ("storage upload failed", b"amazon s3"),
                          ("notary API unavailable", b"service unavailable")):
        if marker in raw.lower():
            return label
    return f"exit {result.returncode}"


def summarize_issues(log):
    issues = log.get("issues")
    if not issues:
        return []
    if not isinstance(issues, list):
        return [{"category": "other notarization issue", "severity": "unknown"}]
    summary = []
    for issue in issues[:25]:
        if not isinstance(issue, dict):
            summary.append({"category": "other notarization issue", "severity": "unknown"})
            continue
        message = issue.get("message", "")
        kind = next((label for marker, label in NOTARY_ISSUES.items()
                     if isinstance(message, str) and marker.lower() in message.lower()), "other notarization issue")
        severity = issue.get("severity")
        summary.append({"category": kind, "severity": severity if severity in ("error", "warning") else "unknown"})
    return summary


def notarize(label, path, credentials, evidence):
    command = ["xcrun", "notarytool"]
    result = capture([*command, "submit", path, "--wait", "--no-progress", "--output-format", "json", *credentials])
    submission = notary_result_json(result)
    try:
        request = str(UUID(submission["id"]))
    except (KeyError, ValueError, TypeError, AttributeError):
        raise ValueError(f"{label} notarization did not return a request ID ({notary_failure(result)})") from None

    submitted_status = submission.get("status")
    status = submitted_status if submitted_status in NOTARY_STATES else "Unknown"
    log_result = capture([*command, "log", request, *credentials])
    log = notary_result_json(log_result)
    try:
        matches = str(UUID(log["jobId"])) == request
    except (KeyError, ValueError, TypeError, AttributeError):
        matches = False
    if not log_result.returncode and matches:
        status = log.get("status") if log.get("status") in NOTARY_STATES else "Unknown"
    record = {"id": request, "status": status, "logRetrieved": not log_result.returncode and matches,
              "issues": summarize_issues(log) if matches else []}
    prior = json.loads(evidence.read_text()) if evidence.exists() else {}
    prior[label] = record
    evidence.write_text(json.dumps(prior, indent=2) + "\n")
    print(f"{label} notarization {status}, request {request}.")
    if (result.returncode or submitted_status != "Accepted" or status != "Accepted"
            or not record["logRetrieved"] or log.get("issues")):
        reason = "incomplete acceptance or log"
        if result.returncode:
            reason = notary_failure(result)
        elif log_result.returncode:
            reason = "log retrieval: " + notary_failure(log_result)
        raise ValueError(f"{label} notarization did not return clean Accepted proof ({reason})")


def check_notary_auth(work):
    credentials = notary_credentials(work)
    result = capture(["xcrun", "notarytool", "history", "--no-progress", "--output-format", "json", *credentials])
    if result.returncode:
        raise ValueError(f"Apple notarization credential check failed ({notary_failure(result)})")
    print("Apple notarization credentials can reach the service.")


def staple(label, path):
    for verb in ("staple", "validate"):
        checked(f"{label} ticket {verb}", ["xcrun", "stapler", verb, path])


def gatekeeper(label, path, kind):
    args = ["spctl", "--assess", "--type", kind, "--verbose"]
    if kind == "open":
        args.extend(("--context", "context:primary-signature"))
    output, errors = checked(f"{label} Gatekeeper", [*args, path])
    if not re.search(rb": accepted\r?\n", output + b"\n" + errors + b"\n"):
        raise ValueError(f"{label} Gatekeeper acceptance was not explicit")
    return "accepted"


def policy_signature(payload, work, public):
    seed = base64.b64decode(os.environ.pop("POWERMOVE_UPDATE_POLICY_KEY").strip(), validate=True)
    if len(seed) != 32:
        raise ValueError("Invalid policy signing seed")
    private = work / "policy.der"
    message = work / "policy-payload"
    private.write_bytes(bytes.fromhex("302e020100300506032b657004220420") + seed)
    try:
        openssl = "/opt/homebrew/opt/openssl@3/bin/openssl"
        key, _ = checked("Policy public-key verification",
                         [openssl, "pkey", "-in", private, "-inform", "DER", "-pubout", "-outform", "DER"])
        expected = bytes.fromhex("302a300506032b6570032100") + base64.b64decode(public, validate=True)
        if key != expected:
            raise ValueError("The policy signing key does not match the bundled key")
        message.write_bytes(DOMAIN + payload)
        signature, _ = checked("Policy signing", [
            openssl, "pkeyutl", "-sign", "-rawin", "-keyform", "DER", "-inkey", private, "-in", message,
        ])
        return base64.b64encode(signature).decode()
    finally:
        private.unlink()


def generate_updates(stage, work, archive, version):
    public_keys = json.loads((HERE / "release/public-keys.json").read_text())
    key = os.environ.pop("SPARKLE_ED_PRIVATE_KEY").strip().encode()
    sign_update = stage / "sparkle/bin/sign_update"
    signature, _ = checked("Sparkle archive signing", [sign_update, "--ed-key-file", "-", "-p", archive], data=key + b"\n")
    sparkle_signature = signature.decode().strip()
    if len(base64.b64decode(sparkle_signature, validate=True)) != 64:
        raise ValueError("Invalid Sparkle archive signature")
    # Verify independently against the key pinned in Info.plist.
    openssl = "/opt/homebrew/opt/openssl@3/bin/openssl"
    sparkle_key = work / "sparkle.der"
    sparkle_key.write_bytes(bytes.fromhex("302a300506032b6570032100") +
                            base64.b64decode(public_keys["sparklePublicKey"], validate=True))
    signature_file = work / "sparkle.sig"
    signature_file.write_bytes(base64.b64decode(sparkle_signature, validate=True))
    checked("Sparkle public-key verification", [
        openssl, "pkeyutl", "-verify", "-pubin", "-keyform", "DER", "-inkey", sparkle_key,
        "-rawin", "-in", archive, "-sigfile", signature_file,
    ])

    archive_url = f"{RELEASE_ROOT}/{version}/Powermove-Native-{version}-mac-arm64.zip"
    size = archive.stat().st_size
    XML.register_namespace("sparkle", SPARKLE)
    rss = XML.Element("rss", version="2.0")
    channel = XML.SubElement(rss, "channel")
    XML.SubElement(channel, "title").text = "Powermove"
    item = XML.SubElement(channel, "item")
    for name, value in (("version", version), ("shortVersionString", version), ("minimumSystemVersion", "14.0")):
        XML.SubElement(item, f"{{{SPARKLE}}}{name}").text = value
    # No sparkle:channel element means Sparkle's default stream. The policy calls it latest.
    XML.SubElement(item, "enclosure", {
        "url": archive_url, "length": str(size), f"{{{SPARKLE}}}edSignature": sparkle_signature,
        "type": "application/octet-stream",
    })
    XML.ElementTree(rss).write(stage / "appcast.xml", encoding="utf-8", xml_declaration=True)

    release = dict(schema=1, channel="latest", version=version, sparkleVersion=version, architecture="arm64",
                   bundleId="com.zellzoi.powermove", minimumMacos=14, archiveUrl=archive_url, size=size,
                   sha256=digest(archive), sparkleSignature=sparkle_signature,
                   expiresUnix=int(time.time()) + 30 * 24 * 3600)
    payload = json.dumps(release, separators=(",", ":")).encode()
    envelope = {"payload": payload.decode(), "signature": policy_signature(payload, work, public_keys["policyPublicKey"])}
    (stage / "policy.json").write_text(json.dumps(envelope) + "\n")


def run_release(stage, work):
    version = os.environ["PM_NATIVE_VERSION"]
    electron_tag = os.environ["PM_ELECTRON_TAG"]
    publish = os.environ.get("PM_NATIVE_PUBLISH") == "true"
    if publish and (os.environ.get("GITHUB_EVENT_NAME") != "workflow_dispatch"
                    or os.environ.get("GITHUB_REF") != f"refs/tags/native-v{version}"):
        raise ValueError("Native release files can be retained only by the approved tag run")
    verify_release_versions(version, version, electron_tag)
    app = stage / "Powermove.app"
    metadata = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    expected = info("latest", version, version)
    for key in ("CFBundleIdentifier", "CFBundleVersion", "CFBundleShortVersionString"):
        if metadata[key] != expected[key]:
            raise ValueError("Bundle identity or version differs from the reviewed metadata")

    p12 = work / "certificate.p12"
    keychain = work / "native-signing.keychain-db"
    keychain_password = secrets.token_hex(24)
    p12.write_bytes(base64.b64decode("".join(os.environ.pop("APPLE_CERTIFICATE_P12").split()), validate=True))
    credentials = notary_credentials(work)
    evidence = stage / "evidence"
    evidence.mkdir()
    notarization_log = evidence / "notarization.json"
    checked("Temporary keychain creation", ["security", "create-keychain", "-p", keychain_password, keychain])
    try:
        checked("Temporary keychain setup", ["security", "set-keychain-settings", "-lut", "7200", keychain])
        checked("Temporary keychain unlock", ["security", "unlock-keychain", "-p", keychain_password, keychain])
        password = os.environ.pop("APPLE_CERTIFICATE_PASSWORD")
        checked("Developer ID import", ["security", "import", p12, "-P", password, "-T", "/usr/bin/codesign",
                                       "-t", "cert", "-f", "pkcs12", "-k", keychain])
        p12.unlink()
        checked("Signing key access", ["security", "set-key-partition-list", "-S", "apple-tool:,apple:,codesign:",
                                       "-s", "-k", keychain_password, keychain])
        checked("Temporary keychain search list", ["security", "list-keychains", "-d", "user", "-s", keychain])
        identities, _ = checked("Developer ID lookup", ["security", "find-identity", "-v", "-p", "codesigning", keychain])
        if identities.count(IDENTITY.encode()) != 1:
            raise ValueError("Expected one valid Developer ID Application identity")

        sign_env = {**os.environ, "PM_CODESIGN_IDENTITY": IDENTITY}
        checked("Hardened runtime signing", [sys.executable, HERE / "package.py", "sign", app], env=sign_env)
        checked("App codesign verification", ["codesign", "--verify", "--deep", "--strict", app])
        runtimes = verify_runtime_versions(app, work)

        submit = stage / "app-for-notary.zip"
        checked("App notarization archive", [sys.executable, HERE / "package.py", "transition-zip", app, submit])
        notarize("app", submit, credentials, notarization_log)
        submit.unlink()
        staple("App", app)
        app_gatekeeper = gatekeeper("App", app, "execute")
        checked("Stapled app codesign verification", ["codesign", "--verify", "--deep", "--strict", app])

        dmg = stage / f"Powermove-Native-{version}-mac-arm64.dmg"
        checked("DMG assembly", ["sh", HERE / "build-dmg.sh", app, dmg])
        checked("DMG signing", ["codesign", "--force", "--timestamp", "--sign", IDENTITY, dmg])
        notarize("dmg", dmg, credentials, notarization_log)
        staple("DMG", dmg)
        dmg_gatekeeper = gatekeeper("DMG", dmg, "open")
        checked("DMG codesign verification", ["codesign", "--verify", "--strict", dmg])
        dmg_hash = digest(dmg)
        archive = stage / f"Powermove-Native-{version}-mac-arm64.zip"
        checked("Sparkle update ZIP", [sys.executable, HERE / "package.py", "transition-zip", app, archive])
        generate_updates(stage, work, archive, version)
        if not publish:
            dmg.unlink()
            archive.unlink()
    finally:
        checked("Temporary keychain cleanup", ["security", "delete-keychain", keychain])

    proof = dict(bundleId=metadata["CFBundleIdentifier"], version=version, bundleBuild=version,
                 previousElectronTag=electron_tag, architecture="arm64", runtimes=runtimes,
                 appGatekeeper=app_gatekeeper, dmgGatekeeper=dmg_gatekeeper, codesign="verified",
                 appStapler="validated", dmgStapler="validated", dmgSha256=dmg_hash,
                 updateMetadata="generated and signed", mode="publish" if publish else "dry-run")
    # Only serialize expected fields after their strict parsers or exact comparisons.
    (evidence / "verification.json").write_text(json.dumps(proof, indent=2) + "\n")
    print("Developer ID app and DMG accepted, stapled, and verified; signed update metadata generated.")


if __name__ == "__main__":
    import sys
    try:
        os.umask(0o077)
        with tempfile.TemporaryDirectory(prefix="signing-", dir=os.environ["RUNNER_TEMP"]) as scratch:
            if sys.argv[1:] == ["--check-notary-auth"]:
                check_notary_auth(Path(scratch))
            elif not sys.argv[1:]:
                run_release(Path(os.environ["PM_NATIVE_STAGE"]), Path(scratch))
            else:
                raise ValueError("unexpected native signing arguments")
    except ValueError as error:
        # checked() only exposes our fixed labels and exit codes. JSON parser
        # messages carry offsets only. No raw subprocess output is forwarded.
        print(f"Native signing failed ({error})", file=sys.stderr)
        sys.exit(1)
    except (OSError, KeyError, subprocess.SubprocessError, TypeError) as error:
        print(f"Native signing failed ({type(error).__name__})", file=sys.stderr)
        sys.exit(1)
