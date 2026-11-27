# Powermove taffy 0.13.0

Upstream: https://github.com/DioxusLabs/taffy, release v0.13.0 as published on
crates.io (the version GPUI-CE pins). License: MIT, in LICENSE. The examples,
dev-dependencies and the release profile are left out of Cargo.toml.

## Patch: measure cache (from taffy 0.14.0)

`src/tree/cache.rs` only. 0.12 made cache keys exact but kept one slot per kind
of query, so two queries of one kind (a flex column sizing a child, then laying
it out at the size it found) replaced each other, and every level of nested
flex columns measured its whole subtree again: twice the work per level. A
Settings page of a few hundred rows took over half a second to lay out.

Taken from 0.14.0: a measure entry is found by its exact key anywhere among the
nine, a new one replaces the least recently used (second-chance eviction), and
a hit must have been measured for the same axis. The use bits are a `Cell` so
`Cache::get` keeps 0.13's `&self` signature.

Upstream's layout fixtures at v0.13.0 (5525 tests) and unit tests pass with
the patch. Drop this folder when GPUI-CE moves to taffy 0.14 or newer.
