# Packaging record, 0.1.0 preview

Recorded 2026-11-12 for ticket #24. Nothing here was published.

## Build host

The host is not the release target. It is Linux aarch64 under PRoot (kernel string `6.17.0-PRoot-Distro`) running Ubuntu 26.04.1 LTS with glibc 2.43. Docker, Podman, qemu-x86_64 and an x86_64 cross compiler are not available.

| Tool | Version |
| --- | --- |
| rustc | 1.94.0 (4a4ef493e 2026-03-02), LLVM 21.1.8, host `aarch64-unknown-linux-gnu` |
| cargo | 1.94.0 (85eff7c80 2026-01-15) |
| C compiler | GCC 15.2.0 |
| maturin | 1.16.0 |
| uv | 0.13.7 |
| CPython | 3.12.16 and 3.13.18 (uv-managed GNU aarch64 builds) |

## Locked Rust build

`cargo build --locked` (dev profile) and `cargo build --locked --release --examples` both succeeded against the committed `Cargo.lock`. The full test suite also runs with `--locked`. No `target-cpu=native` or other target-specific flags are set; the repository has no `.cargo/config.toml`.

## Source distribution

`maturin sdist` builds `iresiphi-0.1.0.tar.gz`. `Cargo.toml` lists what goes in through its `include` key, which both `cargo package` and maturin read. The archive contains:

- `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` and `pyproject.toml`
- the Rust sources (`src/`), examples and tests, including `tests/fixtures/two_suppliers.parity.json`
- the Python package sources (`python/iresiphi/`)
- `scripts/generate_fixture.py` and `scripts/third_party_notices.py`
- `README.md`, `LICENSE`, `THIRD_PARTY_NOTICES.md`, `docs/preview.md` and `docs/measurements/`
- `PKG-INFO`, which declares `License-Expression: Apache-2.0` and both license files

The archive does not include the vendored agent skills (`.claude/`, `.agents/`), `AGENTS.md`, `CLAUDE.md`, the planning specs under `docs/spec/`, or the research and design notes. Before this ticket the archive included all of them, 240 skill files among them. It contains no data beyond the synthetic test fixtures, and no credentials.

Building from the sdist needs Rust 1.94.0 and a C compiler. maturin 1.16.0 is the PEP 517 backend named in `pyproject.toml`. The wheels below were built from the extracted sdist, not from the repository checkout, so the sdist is known to contain everything the build needs.

SDIST_DETAILS

## Wheels

Built with `maturin build --locked --profile dev --compatibility linux --auditwheel skip` for each interpreter. The dev profile was used for the wheels (unoptimized, no debug info). The plain `linux` tag is deliberate: nothing on this host can certify manylinux compliance.

WHEEL_DETAILS

Each wheel contains the `iresiphi` Python package, the native module `iresiphi/_native.cpython-3XX-aarch64-linux-gnu.so`, `LICENSE` and `THIRD_PARTY_NOTICES.md` under `dist-info/licenses/`, and a CycloneDX SBOM that maturin generates (`dist-info/sboms/iresiphi.cyclonedx.json`).

### Local audit of the native module

`readelf -d` and `ldd` on the cp312 module:

| Check | Result |
| --- | --- |
| ELF class and machine | ELF64, AArch64 |
| `NEEDED` | `libgcc_s.so.1`, `libm.so.6`, `libc.so.6`, `ld-linux-aarch64.so.1` |
| RPATH / RUNPATH | none |
| libpython | not linked (PyO3 `extension-module`) |
| glibc symbol versions required | `GLIBC_2.17` up to `GLIBC_2.43` |
| Newest symbols | `acosf`, `asinf`, `atan2f`, `coshf`, `sinhf`, `log10f` at `GLIBC_2.43`; `hypot`, `hypotf` at 2.35; `pthread_create`, `pthread_join`, `dlsym` and others at 2.34; `stat64`, `fstat64`, `fstatat` at 2.33; `exp`, `log`, `pow`, `log2` at 2.29 |

The module links only to the C runtime, and nothing beyond the libraries the manylinux policy allows. Its symbol versions, though, are bound to the build host's glibc 2.43. A manylinux_2_28 wheel may require at most `GLIBC_2.28`, so these artifacts cannot be relabelled; the release wheels must be built inside a manylinux_2_28 image, whose older glibc gives the older symbol versions.

### Installation and tests

Each wheel was installed offline (`uv pip install --offline --no-index`) into a fresh `uv` virtual environment for its interpreter. The Python suites (`python_acceptance`, `python_handles`, `python_cancellation`) and `examples/run.py` were run against the installed package; see the ticket's final report for the counts.

## External blockers

These release gates cannot be met on this host. They need an x86_64 Linux machine or CI runner with a container runtime:

1. Build cp312 and cp313 wheels in a manylinux_2_28 x86_64 image pinned by digest (for example `quay.io/pypa/manylinux_2_28_x86_64@sha256:...`) with maturin 1.16.0 and Rust 1.94.0. No digest is recorded because no image was pulled.
2. Run `auditwheel show` and `auditwheel repair --plat manylinux_2_28_x86_64` on each wheel and keep the reports.
3. Install each wheel on a clean image with glibc 2.28 (for example AlmaLinux 8 or Rocky Linux 8), import it, and run the Python suites there.
4. Check crate and PyPI name availability and account ownership immediately before any publication.
