// V1's own updater reads the V2 release: electron-updater (the exact version V1
// ships, 6.8.3) resolves the release through its GitHub provider against a
// loopback server laid out like a GitHub release, picks the file its
// MacUpdater would pick, and downloads it with its own SHA-512 check.
//
//   node electron_feed_check.cjs <node_modules/electron-updater> <release-assets-dir> <version>
//
// Assets: latest-mac.yml, beta-mac.yml and the ZIP they name. Prints one line
// per case and exits non-zero on any mismatch. Never contacts the network.
"use strict";
const http = require("http");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { createRequire } = require("module");

const [updaterDir, assets, version] = process.argv.slice(2);
if (!updaterDir || !assets || !version) {
  console.error("usage: electron_feed_check.cjs <electron-updater dir> <assets dir> <version>");
  process.exit(2);
}
const load = createRequire(path.join(path.resolve(updaterDir), "package.json"));
const pkg = load("./package.json");
const { GitHubProvider } = load("./out/providers/GitHubProvider");
const { findFile } = load("./out/providers/Provider");
const { HttpExecutor, CancellationToken, configureRequestUrl, configureRequestOptions } = load("builder-util-runtime");
const semver = load("semver");

const owner = "iterative-computer";
const repo = "powermove";
const tag = `v${version}`;
const base = `/${owner}/${repo}/releases`;

class NodeExecutor extends HttpExecutor {
  createRequest(options, callback) {
    return http.request(options, callback);
  }
  // ElectronHttpExecutor.download (out/electronHttpExecutor.js): a file download
  // through HttpExecutor's pipes, where DigestTransform checks the SHA-512.
  download(url, destination, options) {
    return options.cancellationToken.createPromise((resolve, reject, onCancel) => {
      const requestOptions = { headers: options.headers || undefined, redirect: "manual" };
      configureRequestUrl(url, requestOptions);
      configureRequestOptions(requestOptions);
      this.doDownload(requestOptions, {
        destination, options, onCancel, responseHandler: null,
        callback: (error) => (error == null ? resolve(destination) : reject(error)),
      }, 0);
    });
  }
}

function serve() {
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    const send = (status, body, type = "application/octet-stream") => {
      res.writeHead(status, { "content-type": type });
      res.end(body);
    };
    if (url.pathname === `${base}.atom`) {
      return send(200, `<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom"><entry><id>tag:github.com,2008:Repository/1/${tag}</id>
<link rel="alternate" type="text/html" href="https://github.com${base}/tag/${tag}"/>
<title>Powermove ${version}</title><content type="html">notes</content></entry></feed>`, "application/atom+xml");
    }
    if (url.pathname === `/api/v3/repos/${owner}/${repo}/releases/latest`) {
      return send(200, JSON.stringify({ tag_name: tag }), "application/json");
    }
    const prefix = `${base}/download/${tag}/`;
    if (url.pathname.startsWith(prefix)) {
      const file = path.join(assets, path.basename(decodeURIComponent(url.pathname.slice(prefix.length))));
      if (fs.existsSync(file)) return send(200, fs.readFileSync(file));
    }
    send(404, "not found", "text/plain");
  });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve(server)));
}

// MacUpdater.doDownloadUpdate (electron-updater 6.8.3, out/MacUpdater.js
// lines 69-76): Apple silicon keeps the arm64 files when there are any.
function macFiles(files, isArm64Mac) {
  const isArm64 = (file) => file.url.pathname.includes("arm64") || file.info.url?.includes("arm64");
  if (isArm64Mac && files.some(isArm64)) return files.filter((file) => isArm64Mac === isArm64(file));
  return files.filter((file) => !isArm64(file));
}
// AppUpdater.checkIfUpdateSupported: minimumSystemVersion against os.release().
function supported(info, darwin) {
  return !info.minimumSystemVersion || !semver.lt(darwin, info.minimumSystemVersion);
}

async function check(server, current, darwin) {
  const executor = new NodeExecutor();
  const updater = {
    allowPrerelease: current.includes("-beta."), // V1's updates.ts
    channel: null,
    currentVersion: new semver.SemVer(current),
    fullChangelog: false,
  };
  const { port } = server.address();
  const provider = new GitHubProvider(
    { provider: "github", owner, repo, host: `127.0.0.1:${port}`, protocol: "http" },
    updater,
    { executor, platform: "darwin", isUseMultipleRangeRequest: false },
  );
  const info = await provider.getLatestVersion();
  const available = semver.gt(info.version, updater.currentVersion) && supported(info, darwin);
  if (!available) return { available, info };
  const zip = findFile(macFiles(provider.resolveFiles(info), true), "zip", ["pkg", "dmg"]);
  if (!zip) throw new Error("no ZIP for Apple silicon");
  const destination = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "pm-feed-")), "update.zip");
  try {
    await executor.download(zip.url, destination, {
      sha512: zip.info.sha512,
      cancellationToken: new CancellationToken(),
      headers: {},
    });
    const bytes = fs.statSync(destination).size;
    return { available, info, zip: path.basename(zip.url.pathname), bytes, channel: provider.channel };
  } finally {
    fs.rmSync(path.dirname(destination), { recursive: true, force: true });
  }
}

(async () => {
  console.log(`electron-updater ${pkg.version}`);
  const server = await serve();
  let failed = false;
  const expect = (label, ok, detail) => {
    console.log(`${ok ? "ok  " : "FAIL"} ${label}: ${detail}`);
    failed ||= !ok;
  };
  try {
    const zipName = `Powermove-${version}-mac-arm64.zip`;
    const size = fs.statSync(path.join(assets, zipName)).size;
    for (const current of ["1.3.2", "1.3.0-beta.2"]) {
      const result = await check(server, current, "25.0.0"); // macOS 26
      expect(`V1 ${current} on macOS 26`, result.available && result.zip === zipName && result.bytes === size,
        `offers ${result.info.version}, downloads ${result.zip} (${result.bytes} bytes, SHA-512 checked by electron-updater)`);
    }
    const sonoma = await check(server, "1.3.2", "23.0.0");
    expect("V1 1.3.2 on macOS 14", sonoma.available, `offers ${sonoma.info.version}`);
    const ventura = await check(server, "1.3.2", "22.6.0");
    expect("V1 1.3.2 on macOS 13 (V2 needs 14)", !ventura.available,
      `stays on V1 (minimumSystemVersion ${ventura.info.minimumSystemVersion})`);
    const newer = await check(server, version, "25.0.0");
    expect(`V2 ${version} itself`, !newer.available, "no update to the same version");
  } catch (error) {
    expect("feed", false, error.stack || String(error));
  } finally {
    server.close();
  }
  process.exit(failed ? 1 : 0);
})();
