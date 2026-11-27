# Document object storage

`pm-model` stores each object's values in a `Vec<V>` in key order and its member
names in an immutable `Arc<KeySchema>`. Objects with the same keys can share
that schema, including objects with different value types. This avoids one
`JsString` slot per repeated member name. Arrays and object nodes retain their
existing outer `Arc` copy-on-write behavior.

Keys keep their exact WTF-8 bytes and byte ordering. Collecting duplicate keys
keeps the last value. Canonical serialization still applies its separate
UTF-16 ordering. Value edits preserve the schema; adding or removing a member
replaces it without changing snapshots. Arbitrary extension members remain
ordinary JSON members.

The process-wide interner holds at most 1,024 weak schema references. Only
schemas with at most 32 keys and at most 4,096 aggregate key bytes are eligible.
FIFO eviction and hash collisions can reduce sharing, but cannot change keys:
every hit checks the exact key sequence. Weak references do not retain the
schema's key vector after its last document owner drops. Empty objects share
one constant empty schema. Wide objects use an uninterned schema and retain
amortized vector growth on insertion.

On 64-bit arm64, `JsString` is 24 bytes, `Node` is 32 bytes, and `ObjectMap<Node>`
is 32 bytes (the former pair-vector map was 24 bytes). For shared shapes, the
requested allocation reduction is approximately `24 * members - 8 * objects`,
less the unique schemas and bounded interner. `ObjectMap::storage_estimate`
exposes schema identity so estimates can count each shared allocation once.
These are allocation estimates, not RSS: allocator size classes, free pages,
temporary buffers, and other subsystems also affect resident memory.

A standalone optimized Rust allocation probe with 96,000 synthetic ten-member
keyframes, each containing two two-member ease objects, retained 89,856,000
requested bytes with pair vectors and 59,904,540 with shared schemas. Construction
took 38 ms and 134 ms respectively on that run. This demonstrates the space/time
tradeoff for repeated shapes; it is not a production-project memory acceptance
result or a guarantee of proportional RSS savings.
