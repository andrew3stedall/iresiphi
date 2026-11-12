"""Iresiphi: source-preserving local recipes over the shared Rust core.

Rust owns validation, planning, serialization, execution and diagnostics. This
package is a thin layer over opaque handles:

* ``Recipe`` — a validated recipe. Load it from a ``.iresiphi`` document or
  from data built with ``iresiphi.build``; ``to_json``/``save`` write the same
  document Rust writes, and ``hash`` is the same canonical SHA-256.
* ``ExecutionContext.run`` (or ``iresiphi.run``) — executes with the interpreter
  released and returns a ``RunResult``. Failures raise ``IresiphiError``;
  cancellation raises ``RunCancelled``; Ctrl-C raises ``RunInterrupted`` (a
  ``KeyboardInterrupt``). Each carries the run report.
* ``RunResult`` — bounded iteration over copied batches (at most 8192 rows each)
  of exact Python values: ``int``, ``decimal.Decimal``, ``datetime.date``,
  ``str``, ``bool`` and ``None``. Use ``close()`` or ``with``; access after close
  raises ``HandleClosedError`` (code ``HANDLE_CLOSED``), repeated close is
  harmless, and values already returned stay valid.
"""

import os

from . import build
from . import _native
from .errors import HandleClosedError, IresiphiError, RunCancelled, RunInterrupted

__all__ = [
    "Recipe",
    "Catalog",
    "ExecutionContext",
    "RunResult",
    "CancellationToken",
    "IresiphiError",
    "HandleClosedError",
    "RunCancelled",
    "RunInterrupted",
    "build",
    "validate",
    "explain",
    "run",
    "load",
    "save",
    "read_bundle",
    "cleanup_staging",
    "from_tagged",
    "FORMAT_VERSION",
    "MAX_BATCH_ROWS",
]

__version__ = _native.__version__
FORMAT_VERSION = _native.FORMAT_VERSION
MAX_BATCH_ROWS = _native.MAX_BATCH_ROWS

Recipe = _native.Recipe
Catalog = _native.Catalog
ExecutionContext = _native.ExecutionContext
RunResult = _native.RunResult
CancellationToken = _native.CancellationToken
read_bundle = _native.read_bundle
cleanup_staging = _native.cleanup_staging
from_tagged = _native.from_tagged


def load(path):
    """Open a saved ``.iresiphi`` document. Rust validates it fully before return."""
    with open(os.fspath(path), "r", encoding="utf-8") as handle:
        return Recipe.from_json(handle.read())


def save(recipe, path):
    """Write ``recipe`` as a ``.iresiphi`` document (exact UTF-8 JSON from Rust)."""
    text = recipe.to_json()
    with open(os.fspath(path), "x", encoding="utf-8") as handle:
        handle.write(text)


def validate(recipe, bindings=None, policy=None, data_checks="none"):
    """Validation report: static, or bound with ``data_checks`` ``none``/``full``."""
    return recipe.validate(bindings, policy, data_checks)


def explain(recipe, bindings=None, policy=None):
    return recipe.explain(bindings, policy)


def run(recipe, bindings, policy, cancel=None, destination=None):
    """Run on a fresh ``ExecutionContext``; see ``ExecutionContext.run``."""
    return ExecutionContext().run(recipe, bindings, policy, cancel=cancel,
                                  destination=destination)
