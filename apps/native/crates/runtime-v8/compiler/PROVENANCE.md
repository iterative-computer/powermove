# Svelte compiler artifact

Unmodified `package/compiler/index.js` and `package/LICENSE.md` extracted from
<https://registry.npmjs.org/svelte/-/svelte-5.56.10.tgz>. The standalone UMD compiler
is executed by V8 while creating the trusted compiler snapshot; no Node loader,
npm install, network download, or JavaScript package manager runs in the app.

- Version: **5.56.10**, matching S1.
- Tarball SHA-256: `5ada56757780fbacdb5959ead0e9c724ec02520d42fed5d0a95995cf1ab1ff2e`.
- Compiler SHA-256: `c60d769e2305be3feea583928d7de10b67e6e594686e4e3508e6da1ca77ffcd2`.
- License: MIT; preserved in `LICENSE.md`.

Native TypeScript-module preprocessing uses OXC **0.152.0** with ES2022 output.
OXC 0.156.0 requires Rust 1.97; 0.152.0 supports the project's Rust 1.96.1.
Compiler output cache identity includes this preprocessing version/target.
