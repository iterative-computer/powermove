# Synthetic Chromium writer oracle

`electron/` contains only newly generated Electron **44.0.0** IndexedDB,
external Blob/File, and localStorage files from `tests/generate-electron.cjs`.
There is no user profile data. The fixture has five media/poster records
(85 payload bytes) and five localStorage entries. `000003.log` files are binary
LevelDB write-ahead journals, not application or operational logs.

The IndexedDB snapshot was captured while the synthetic Electron process was
running, after its write transaction completed. localStorage was captured after
that process quit. Only CURRENT, manifest, journal and synthetic blob bytes are
included; machine profile/preferences/caches and snapshot metadata are excluded.

Regenerate in a fresh `/tmp/pm-s6-synthetic-*` profile with Electron 44, then use
the CLI's snapshot commands. Copy only the snapshot's `leveldb/` and `blobs/`
subdirectories. Transaction timestamps may differ; record content is synthetic.
Additional SST, fragmented/torn journal, checksum, compression/wrapper, low-space,
proxy, closure and interruption fixtures are generated in test temporary folders.
