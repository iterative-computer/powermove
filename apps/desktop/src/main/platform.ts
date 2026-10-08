import path from 'node:path';

/** Native executable names and npm layouts shared by discovery and packaging. */
export function executableName(name: string, platform: NodeJS.Platform = process.platform): string {
  return platform === 'win32' ? `${name}.exe` : name;
}

export function agentPlatform(platform: NodeJS.Platform = process.platform, arch: string = process.arch) {
  const triples: Record<string, string> = {
    'darwin-arm64': 'aarch64-apple-darwin', 'darwin-x64': 'x86_64-apple-darwin',
    'win32-x64': 'x86_64-pc-windows-msvc', 'win32-arm64': 'aarch64-pc-windows-msvc',
    'linux-x64': 'x86_64-unknown-linux-musl', 'linux-arm64': 'aarch64-unknown-linux-musl'
  };
  const triple = triples[`${platform}-${arch}`];
  if (!triple) throw new Error(`Agent runtimes are unavailable for ${platform}-${arch}.`);
  return { suffix: `${platform}-${arch}`, triple, codex: executableName('codex', platform), claude: executableName('claude', platform) };
}

export function encoderBinary(appPath: string, resourcesPath?: string, platform: NodeJS.Platform = process.platform): string {
  const binary = executableName('ffmpeg', platform);
  return resourcesPath ? path.join(resourcesPath, 'encoder', binary) : path.join(appPath, 'node_modules', 'ffmpeg-static', binary);
}

export function windowChrome(platform: NodeJS.Platform = process.platform): Electron.BrowserWindowConstructorOptions {
  return platform === 'darwin'
    ? { titleBarStyle: 'hiddenInset', trafficLightPosition: { x: 14, y: 15 } }
    : { titleBarStyle: 'hidden', titleBarOverlay: { color: '#ededef', symbolColor: '#171717', height: 44 }, backgroundColor: '#ededef' };
}
