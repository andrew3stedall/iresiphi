#!/usr/bin/env bash
# A fresh, offline container has the wheel and test fixtures, not the build tree.
set -euo pipefail
[[ "$(uname -m)" == x86_64 ]]
abi="$1"
export PATH="/opt/python/$abi/bin:$PATH"
python - <<'PY'
import platform
assert platform.libc_ver() == ('glibc', '2.28'), platform.libc_ver()
print(platform.platform(), platform.python_version(), platform.libc_ver())
PY
python -m venv /tmp/clean-venv
/tmp/clean-venv/bin/python -m pip install --no-index --no-deps /dist/*"$abi"*.whl
cd /tests
/tmp/clean-venv/bin/python -m unittest -v python_acceptance python_handles python_cancellation
/tmp/clean-venv/bin/python - <<'PY'
import iresiphi
print('Installed module:', iresiphi.__file__)
assert iresiphi.__version__ == '0.1.0'
PY
