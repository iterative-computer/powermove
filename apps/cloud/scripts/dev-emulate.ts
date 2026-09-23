import { createEmulator } from 'emulate';

const emulator = await createEmulator({
  service: 'google',
  port: 4002,
  seed: { google: {
    users: [
      { email: 'jude@example.com', name: 'Jude' },
      { email: 'mara@example.com', name: 'Mara' },
    ],
    oauth_clients: [{
      client_id: 'powermove-local',
      client_secret: 'powermove-local-secret',
      name: 'Powermove Local',
      redirect_uris: ['http://localhost:8787/v1/auth/oauth2/callback/google'],
    }],
  } },
});
console.log(`Google emulator discovery: ${emulator.url}/.well-known/openid-configuration`);
process.on('SIGINT', () => { void emulator.close().then(() => process.exit(0)); });
process.on('SIGTERM', () => { void emulator.close().then(() => process.exit(0)); });
await new Promise(() => {});
