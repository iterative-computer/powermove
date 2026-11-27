# Synthetic main data fixtures

Reference: main `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`.
These are invented records, not a user profile, project, path or media capture.
Tests read only fixtures owned by this crate.

- `main-document.json`: disabled Memory, nested gain/mute, source links,
  exact caption/word times, retired web delivery, project workspace, unknown
  fields and a packed history with pending redo and history-only media.
- `disk-gzip.json`: compact JSON of that document, gzip with mtime 0 and a
  base64 payload, in main's `__powermoveDiskValue:1` envelope. `bytes` is the
  uncompressed UTF-8 length. No external decoder executable is required.
- `main-pmv3.pmv`: `PMV3\n`, little-endian u32 JSON-header length, that document
  and a relative media range, then literal `synthetic-range-media` bytes.
  Future header/per-media fields exercise archive retention.
- `main-ranges.pmv`: `PMV4\n`, dead bytes, literal media before the document,
  a gap, document JSON, checked index and the standard 64-byte footer.
  Future root/document/media index fields exercise range metadata retention.

SHA-256 values and footer hashes describe the actual synthetic bytes. The tests
also corrupt bytes and forge an index checksum over overlapping ranges: correct
hashes alone must not make overlapping ranges acceptable.
