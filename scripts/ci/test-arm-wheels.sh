#!/usr/bin/env bash
# Native ARM semantic verification; these plain Linux wheels are not releases.
set -euo pipefail
[[ "$(uname -m)" == aarch64 ]]
evidence="$(pwd)/ci-artifacts/arm-python"
mkdir -p "$evidence/wheels"
exec > >(tee "$evidence/validation.log") 2>&1
git rev-parse HEAD
uname -a
"$IRESIPHI_PY312" --version
"$IRESIPHI_PY313" --version
"$IRESIPHI_PY312" -m pip install 'maturin==1.16.0'
for minor in 12 13; do
  if [[ "$minor" == 12 ]]; then
    interpreter="$IRESIPHI_PY312"
  else
    interpreter="$IRESIPHI_PY313"
  fi
  "$IRESIPHI_PY312" -m maturin build --locked --profile dev \
    --interpreter "$interpreter" --compatibility linux --auditwheel skip \
    --out "$evidence/wheels"
  environment="$(mktemp -d)/venv"
  "$interpreter" -m venv "$environment"
  "$environment/bin/python" -m pip install --no-index --no-deps \
    "$evidence"/wheels/*cp3"$minor"*.whl
  (cd tests && "$environment/bin/python" -m unittest -v \
    python_acceptance python_handles python_cancellation) \
    2>&1 | tee "$evidence/cp3$minor-tests.log"
done
