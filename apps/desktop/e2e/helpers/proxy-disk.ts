/** Proxy tests that are not about the disk guard must not depend on the
 * shared machine's free space (the guard reserves max(20 GiB, 10% of the
 * volume) and pauses generation below it). This reserve applies only to an
 * engine addon built with `build-engine-addon.mjs --test-hooks`; production
 * builds never read it. The dedicated low-disk tests drive the guard with a
 * simulated free-space value instead (`pm-proxy/tests/disk_pause.rs`). */
export const PROXY_TEST_DISK_ENV = { POWERMOVE_TEST_PROXY_DISK_RESERVE_BYTES: String(1 << 30) };
