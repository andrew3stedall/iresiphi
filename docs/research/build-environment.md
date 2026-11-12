# Preview build environment verification

Verified 2026-11-11 19:25 UTC during implementation. These observations establish available tools and published versions, not successful application builds or release certification.

## Exact baseline availability

The crates.io REST endpoint returned HTTP 403 here. The authoritative Cargo sparse registry at `https://index.crates.io/` was accessible and confirmed every selected version is present and not yanked:

| Crate | Exact version | Declared minimum Rust |
| --- | --- | --- |
| datafusion | 55.2.0 | 1.94.0 |
| pyo3 | 0.30.1 | 1.85 |
| calamine | 0.38.0 | 1.88 |
| serde_json | 1.0.154 | 1.71 |
| csv | 1.4.0 | 1.73 |

Sources: [DataFusion index](https://index.crates.io/da/ta/datafusion), [PyO3 index](https://index.crates.io/py/o3/pyo3), [calamine index](https://index.crates.io/ca/la/calamine), [serde_json index](https://index.crates.io/se/rd/serde_json), [csv index](https://index.crates.io/3/c/csv). [Maturin 1.16.0 PyPI metadata](https://pypi.org/pypi/maturin/1.16.0/json) and the [Rust 1.94.0 channel manifest](https://static.rust-lang.org/dist/channel-rust-1.94.0.toml) were also fetched successfully. No baseline substitution or corrective version decision is needed.

Additional non-yanked stable versions inspected for implementation: num-bigint 0.4.8, num-traits 0.2.19, sha2 0.11.1, tokio 1.54.1, chrono 0.4.45, zip 9.5.1, tempfile 3.27.0, libc 0.2.191, serde 1.0.229. Their registry-declared minimum Rust versions do not exceed 1.94. This is availability evidence; use the eventual Cargo.lock for the actual dependency graph.

## Provisioned local tools

Rustup installed `1.94.0-aarch64-unknown-linux-gnu`, minimal profile. Executed version checks returned `rustc 1.94.0 (4a4ef493e 2026-03-02)` and `cargo 1.94.0 (85eff7c80 2026-01-15)`. Tools reside under `/home/t3/.cargo/bin`; add this directory to PATH. System C compiler is GCC 15.2.0.

Downloaded the published GNU aarch64 wheels for uv 0.13.7 and maturin 1.16.0, extracted their executables, and executed version checks. Session paths:

- `/tmp/iresiphi-tools/uv/uv-0.13.7.data/scripts/uv`
- `/tmp/iresiphi-tools/maturin/maturin-1.16.0.data/scripts/maturin`

uv installed ordinary GNU aarch64 CPython 3.12.16 and 3.13.18 successfully:

- `/home/t3/.local/share/uv/python/cpython-3.12-linux-aarch64-gnu/bin/python3.12`
- `/home/t3/.local/share/uv/python/cpython-3.13-linux-aarch64-gnu/bin/python3.13`

Use explicit interpreter paths. Initially `python` resolved to an Android/Termux CPython 3.13.13, whereas `/usr/bin/python3` was Ubuntu CPython 3.14.4. Neither is an appropriate substitute for the installed GNU 3.12/3.13 test interpreters. System Python lacked ensurepip; uv avoids modifying system Python.

## Limits on evidence from this machine

This machine is **aarch64**, running Ubuntu 26.04.1 through PRoot, glibc 2.43. It is not the supported release target Linux x86_64 manylinux_2_28. Docker, Podman, qemu-x86_64 and an x86_64 cross compiler were not present. Local GNU aarch64 tests can establish semantic and binding behavior, but cannot certify x86_64 wheels, minimum glibc compatibility, clean container installation or a pinned build-image digest. Those remain explicit release gates; do not relabel a local artifact as a supported distribution.

Build with `CARGO_BUILD_JOBS=1`, development debug information disabled and incremental compilation disabled to reduce memory pressure. No application build, dependency audit, performance, wheel audit or test result is asserted by this note.

## Python bindings build and test (ticket #23, 2026-11-12)

Observed on the same aarch64 PRoot machine (4 cores, ~2 GB free RAM). These are local semantic results only; no Linux x86_64 manylinux_2_28 wheel was built, audited or installed on a clean image here.

Tools reused from the earlier session, still present and version-checked (`maturin 1.16.0`, `uv 0.13.7 (aarch64-unknown-linux-musl)`):

- `/tmp/iresiphi-tools/maturin/maturin-1.16.0.data/scripts/maturin` (sha256 `ac34e9599a28f134712dd70e53078624e9d786efde1b8fcfd1bbbe7f48f30baf`)
- `/tmp/iresiphi-tools/uv/uv-0.13.7.data/scripts/uv` (sha256 `f4752297fa474760b9db6586501db3ee5dd226ce49d9c0c23d6f47930d44f30e`)

`/tmp` does not survive a reboot. If the tools are gone, download the published GNU aarch64 wheels from PyPI, check them against the sha256 in `https://pypi.org/pypi/<name>/<version>/json`, and unzip the executable.

Shared build environment. Every cargo or maturin command needs the codegen-units override, which keeps this worktree's `iresiphi` artifacts apart from other worktrees that use the same target directory:

```sh
export PATH=$HOME/.cargo/bin:$PATH CARGO_BUILD_JOBS=2 \
  CARGO_TARGET_DIR=/home/t3/.t3/worktrees/iresiphi/t3-f0588568/target
CFG='profile.dev.package.iresiphi.codegen-units=204'   # reserved per worktree
```

Virtual environments without pip. `uv venv` works, and so does `python3.13 -m venv --without-pip`:

```sh
uv venv --python /home/t3/.local/share/uv/python/cpython-3.12-linux-aarch64-gnu/bin/python3.12 venv312
/home/t3/.local/share/uv/python/cpython-3.13-linux-aarch64-gnu/bin/python3.13 -m venv --without-pip venv313
```

Wheels are built with the dev profile. A release-profile DataFusion build was not attempted because of the memory risk on this machine. Use `--auditwheel skip` and the plain `linux` tag, since nothing here can certify manylinux:

```sh
maturin build --profile dev -i venv312/bin/python --out wheels \
  --compatibility linux --auditwheel skip --config "$CFG"
uv pip install --python venv312/bin/python --offline --no-index --reinstall \
  wheels/iresiphi-0.1.0-cp312-cp312-linux_aarch64.whl
```

Tests run from `tests/` against the installed wheel, using only the stdlib `unittest`, with no pytest dependency:

```sh
cd tests && ../venv312/bin/python -m unittest -v python_acceptance python_handles python_cancellation
```

The Rust half of the parity golden is `cargo test --test python_parity --config "$CFG"`. Regenerate it with `IRESIPHI_BLESS=1` only when a Rust change is intended to alter results.

Notes for workload measurement (#24):

- The dev-profile extension processes about 11 ms per generated invoice line here: 2,000 lines took about 23 s and 20,000 lines about 254 s, measured while a parallel cargo build was running. Optimized-build timings would need a release-profile wheel, which this machine did not build.
- `cargo test --features python` cannot link test binaries, because pyo3's `extension-module` feature leaves libpython unlinked. Python behaviour is tested through the wheel, and `cargo check --features python` is the compile gate.
