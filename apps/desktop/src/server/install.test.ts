import { describe, expect, it } from 'vitest';

import { launchdPlist, systemdUnit, unitPath, unitText, LAUNCHD_LABEL } from './install';

const spec = { entry: '/opt/powermove/bin/powermove.mjs', node: '/usr/bin/node', args: ['--port', '4747', '--exports', '/home/me/My Exports'], userData: '/home/me/.powermove', home: '/home/me' };

describe('service units', () => {
  it('writes a systemd user unit that restarts and logs into the profile', () => {
    const unit = systemdUnit(spec);
    expect(unit).toContain('ExecStart=/usr/bin/node /opt/powermove/bin/powermove.mjs serve --port 4747 --exports "/home/me/My Exports"');
    expect(unit).toContain('Restart=always');
    expect(unit).toContain('StandardOutput=append:/home/me/.powermove/serve.log');
    expect(unit).toContain('WantedBy=default.target');
    expect(unitPath({ ...spec, platform: 'linux' })).toBe('/home/me/.config/systemd/user/powermove.service');
    expect(unitText({ ...spec, platform: 'linux' })).toBe(unit);
  });

  it('writes a launchd agent that keeps the host alive', () => {
    const plist = launchdPlist(spec);
    expect(plist).toContain(`<string>${LAUNCHD_LABEL}</string>`);
    expect(plist).toContain('<string>/home/me/My Exports</string>');
    expect(plist).toContain('<key>KeepAlive</key><true/>');
    expect(unitPath({ ...spec, platform: 'darwin' })).toBe(`/home/me/Library/LaunchAgents/${LAUNCHD_LABEL}.plist`);
    expect(unitText({ ...spec, platform: 'darwin' })).toBe(plist);
  });

  it('escapes what XML and shells care about', () => {
    expect(launchdPlist({ ...spec, args: ['--exports', '/a&b<c'] })).toContain('<string>/a&amp;b&lt;c</string>');
    expect(systemdUnit({ ...spec, args: ['--exports', 'it"s'] })).toContain('"it\\"s"');
  });
});
