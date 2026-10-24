/*
 * Project fingerprints off the window's thread (project-fingerprint.ts): SHA-256
 * over a project's JSON, received as UTF-8 pieces. One digest per job id.
 */
import { serveFingerprints, type FingerprintScope } from './fingerprint-digests';

serveFingerprints(self as unknown as FingerprintScope);
