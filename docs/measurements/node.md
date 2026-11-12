# Node bindings local evidence (#30)

Verified on 2026-11-13 on native aarch64 under PRoot, glibc 2.43:

- Node v24.22.2: `node --test test/*.test.js` in `bindings/node`: 9 passed,
  0 failed, 0 skipped. Includes unchanged Rust/Python golden parity, exact
  decimals and Int64 bounds, structured errors, ownership, bundle delivery,
  pre-aborted and active AbortSignal cancellation.
- TypeScript 5.9.3: `tsc --strict --noEmit --target ES2022 --module node16
  --moduleResolution node16 test/types.ts`: passed, including a negative check
  preventing JavaScript numbers from being typed as returned domain scalars.
- Root: `cargo test --locked --no-fail-fast`: 229 passed, 0 failed.
- Addon: development-profile build with napi 2.16.17, napi-derive 2.16.13 and
  napi-build 2.1.6, using its standalone lockfile.

All Cargo commands used `CARGO_BUILD_JOBS=2`, the shared target directory
`/home/t3/.t3/worktrees/iresiphi/t3-f0588568/target`, and
`--config 'profile.dev.package.iresiphi.codegen-units=207'`.

These are local semantic checks. They do not certify Node 22, hosted ARM,
Linux x86_64, or the hosted Node 24 versions. The four CI matrix passes must be
recorded at the pushed commit. No 100,000-line workload, x86 emulation, release
build, publication or Python wheel rebuild was performed in this session.
