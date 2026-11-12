#!/usr/bin/env bash
# Run only inside the pinned x86 manylinux builder in GitHub Actions.
set -euo pipefail
[[ "$(uname -m)" == x86_64 ]]
mkdir -p /io/ci-artifacts/{dist,audit,inventory}
exec > >(tee /io/ci-artifacts/build-wheels.log) 2>&1
export PATH="/root/.cargo/bin:/opt/python/cp312-cp312/bin:$PATH"
export CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR=/tmp/iresiphi-target
curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o /tmp/rustup-init.sh
sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain 1.94.0
python -m pip install 'maturin==1.16.0'
rustc -Vv
cargo -V
python -V
maturin --version
auditwheel --version
ldd --version
cd /io
maturin sdist --locked --out ci-artifacts/dist
tar -tzf ci-artifacts/dist/iresiphi-0.1.0.tar.gz > ci-artifacts/inventory/sdist.txt
mkdir /tmp/iresiphi-sdist
tar -xzf ci-artifacts/dist/iresiphi-0.1.0.tar.gz -C /tmp/iresiphi-sdist
cd /tmp/iresiphi-sdist/iresiphi-0.1.0
for abi in cp312-cp312 cp313-cp313; do
  interpreter="/opt/python/$abi/bin/python"
  maturin build --locked --release --interpreter "$interpreter" \
    --compatibility manylinux_2_28 --out /tmp/wheels
  for wheel in /tmp/wheels/*"$abi"*.whl; do
    auditwheel show "$wheel" | tee "/io/ci-artifacts/audit/$abi-show.txt"
    auditwheel repair --plat manylinux_2_28_x86_64 \
      --wheel-dir /io/ci-artifacts/dist "$wheel"
  done
done
cd /io
for wheel in ci-artifacts/dist/*.whl; do
  python -m zipfile -l "$wheel" > "ci-artifacts/inventory/$(basename "$wheel").txt"
done
sha256sum ci-artifacts/dist/* > ci-artifacts/SHA256SUMS
