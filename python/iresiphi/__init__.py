"""Local source-preserving recipes backed by the shared Rust implementation."""
import datetime
import decimal
import json
from . import _native

CancellationToken = _native.CancellationToken

def _value(value):
    kind = value["kind"]
    if kind == "null":
        return None
    raw = value["value"]
    if kind == "decimal":
        return decimal.Decimal(raw)
    if kind == "int64":
        return int(raw)
    if kind == "date":
        return datetime.date.fromisoformat(raw)
    return raw

class Recipe:
    def __init__(self, declaration):
        self._handle = _native.Recipe(declaration if isinstance(declaration, str) else json.dumps(declaration))

    @classmethod
    def from_json(cls, text):
        return cls(text)

    def to_json(self):
        return self._handle.to_json()

    def validate(self, bindings=None, policy=None, data_checks="none"):
        if bindings is None and policy is None:
            if data_checks != "none":
                raise ValueError("full checks require bindings and policy")
            return json.loads(self._handle.validate())
        if bindings is None or policy is None:
            raise ValueError("bindings and policy must be supplied together")
        return json.loads(self._handle.validate_bound(json.dumps(bindings), json.dumps(policy), data_checks))

    def explain(self, bindings=None, policy=None):
        if bindings is None and policy is None:
            return json.loads(self._handle.explain())
        if bindings is None or policy is None:
            raise ValueError("bindings and policy must be supplied together")
        return json.loads(self._handle.explain_bound(json.dumps(bindings), json.dumps(policy)))

    def close(self):
        self._handle.close()

class Result:
    def __init__(self, handle):
        self._handle = handle

    @property
    def status(self):
        return self._handle.status

    @property
    def metadata(self):
        return json.loads(self._handle.metadata())

    def batch(self, table, offset=0, count=8192):
        return [{name: _value(value) for name, value in row.items()}
                for row in json.loads(self._handle.batch(table, offset, count))]

    def close(self):
        self._handle.close()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

class ExecutionContext:
    def __init__(self):
        self._handle = _native.ExecutionContext()

    def run(self, recipe, bindings, policy, cancel=None, destination=None):
        return Result(self._handle.run(recipe._handle, json.dumps(bindings),
                                      json.dumps(policy), cancel, destination))

def validate(recipe, bindings=None, policy=None, data_checks="none"):
    return recipe.validate(bindings, policy, data_checks)

def explain(recipe, bindings=None, policy=None):
    return recipe.explain(bindings, policy)

def run(recipe, bindings, policy, cancel=None, destination=None):
    return ExecutionContext().run(recipe, bindings, policy, cancel, destination)
