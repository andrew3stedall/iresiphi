"""Builders for portable recipe declarations.

Each function returns plain data in the ``.iresiphi`` format 0.1 shape. Nothing
here validates or executes: pass the result to ``iresiphi.Recipe`` and Rust
validates it with the same model the Rust API uses, so a recipe built here and
the same recipe built in Rust have the same canonical JSON and hash.

Exact values are tagged strings. ``lit`` accepts ``int``, ``decimal.Decimal``,
``datetime.date``, ``str``, ``bool`` and ``None``; ``float`` is rejected because
binary floating point cannot carry an exact decimal value.
"""

import datetime
import decimal

from .errors import IresiphiError

FORMAT_VERSION = "0.1"


def _non_portable(message):
    return IresiphiError({
        "code": "NON_PORTABLE",
        "phase": "portability",
        "message": message,
        "severity": "error",
        "remediation": "Use int, decimal.Decimal, datetime.date, str, bool or None.",
    })


# -- types -------------------------------------------------------------------

def string_type():
    return {"kind": "string"}


def int64_type():
    return {"kind": "int64"}


def boolean_type():
    return {"kind": "boolean"}


def date_type():
    return {"kind": "date"}


def decimal_type(precision, scale):
    return {"kind": "decimal", "precision": precision, "scale": scale}


# -- tagged literal values -----------------------------------------------------

def _decimal_text(value):
    if not value.is_finite():
        raise _non_portable("NaN and infinite decimals are not values")
    return format(value, "f")


def tagged(value, precision=None, scale=None):
    """The exact tagged transport form of a Python value."""
    if value is None:
        return {"kind": "null"}
    if isinstance(value, bool):
        return {"kind": "boolean", "value": value}
    if isinstance(value, int):
        return {"kind": "int64", "value": str(value)}
    if isinstance(value, str):
        return {"kind": "string", "value": value}
    if isinstance(value, decimal.Decimal):
        text = _decimal_text(value)
        if scale is None:
            scale = max(0, -value.as_tuple().exponent)
        if precision is None:
            integer = text.lstrip("-").split(".")[0].lstrip("0")
            precision = max(1, len(integer) + scale)
        return {"kind": "decimal", "value": text, "precision": precision, "scale": scale}
    if isinstance(value, datetime.datetime):
        raise _non_portable("timestamps are deferred; convert to a declared business date")
    if isinstance(value, datetime.date):
        return {"kind": "date", "value": value.isoformat()}
    if isinstance(value, float):
        raise _non_portable("binary floating-point values are not exact; use decimal.Decimal")
    if callable(value):
        raise _non_portable("host-only callables cannot be saved or executed portably")
    raise _non_portable(f"{type(value).__name__} is not a portable scalar value")


def lit(value, precision=None, scale=None):
    """A literal expression. Decimal precision/scale default to the value's own digits."""
    return {"op": "literal", "value": tagged(value, precision, scale)}


# -- expressions ---------------------------------------------------------------

def ref(source, field, path=()):
    """A field reference, optionally through named relationship ``path`` steps."""
    return {"op": "field", "source": source, "field": field, "path": list(path)}


def feature(id):
    return {"op": "feature", "id": id}


def _binary(op):
    def build(left, right):
        return {"op": op, "left": left, "right": right}
    build.__name__ = op
    return build


add = _binary("add")
subtract = _binary("subtract")
multiply = _binary("multiply")
equal = _binary("equal")
less = _binary("less")
less_equal = _binary("less_equal")
and_ = _binary("and")
or_ = _binary("or")


def not_(expr):
    return {"op": "not", "expr": expr}


def is_null(expr):
    return {"op": "is_null", "expr": expr}


def coalesce(*values):
    return {"op": "coalesce", "values": list(values)}


def case(condition, then, otherwise):
    return {"op": "case", "condition": condition, "then": then, "otherwise": otherwise}


def cast(expr, data_type):
    return {"op": "cast", "expr": expr, "data_type": data_type}


def quantize(expr, precision, scale, rounding="half_even"):
    """Explicit decimal quantization; ``rounding`` is ``half_even`` or ``toward_zero``."""
    return {"op": "quantize", "expr": expr, "precision": precision, "scale": scale,
            "rounding": rounding}


def valid_on(date, valid_from, valid_to):
    return {"op": "valid_on", "date": date, "valid_from": valid_from, "valid_to": valid_to}


def aggregate(relationship, function, field=None, filter=None):
    expr = {"op": "aggregate", "relationship": relationship, "function": function}
    if field is not None:
        expr["field"] = field
    if filter is not None:
        expr["filter"] = filter
    return expr


# -- declarations ----------------------------------------------------------------

def field(path, data_type, nullable=False, header=False):
    """A projected source field."""
    declared = {"path": path, "data_type": data_type}
    if nullable:
        declared["nullable"] = True
    if header:
        declared["header"] = True
    return declared


def source(id, fields, identity, grain=None, array=None):
    projection = {"fields": dict(fields)}
    if array is not None:
        projection["array"] = array
    return {"id": id, "projection": projection, "identity": list(identity),
            "grain": list(identity if grain is None else grain)}


def validity(date, valid_from, valid_to):
    return {"date": date, "valid_from": valid_from, "valid_to": valid_to}


def relationship(id, from_, to, from_keys, to_keys, cardinality, validity=None, strict=False):
    declared = {"id": id, "from": from_, "to": to, "from_keys": list(from_keys),
                "to_keys": list(to_keys), "cardinality": cardinality}
    if validity is not None:
        declared["validity"] = validity
    if strict:
        declared["strict"] = True
    return declared


def feature_declaration(id, expr):
    return {"id": id, "expr": expr}


def column(name, expr):
    return {"name": name, "expr": expr}


def output(id, anchor, columns, eligibility=None, filter=None, detail=False,
           group_by=(), aggregates=()):
    declared = {"id": id, "anchor": anchor, "columns": list(columns)}
    if eligibility is not None:
        declared["eligibility"] = eligibility
    if filter is not None:
        declared["filter"] = filter
    if detail:
        declared["detail"] = True
    if group_by:
        declared["group_by"] = list(group_by)
    if aggregates:
        declared["aggregates"] = list(aggregates)
    return declared


def recipe(name, sources, outputs, relationships=(), features=(),
           required_capabilities=(), annotations=None):
    declared = {"format_version": FORMAT_VERSION, "name": name, "sources": list(sources),
                "relationships": list(relationships), "features": list(features),
                "outputs": list(outputs)}
    if required_capabilities:
        declared["required_capabilities"] = list(required_capabilities)
    if annotations:
        declared["annotations"] = dict(annotations)
    return declared


# -- runtime inputs ------------------------------------------------------------------

def binding(path, adapter, **settings):
    """Bind one source to a local file at run time (never saved in the recipe)."""
    return {"path": path, "adapter": adapter, "settings": settings}


def policy(read_roots, output_roots=(), limits=None, diagnostic_values=False):
    declared = {"allowed_read_roots": list(read_roots),
                "allowed_output_roots": list(output_roots),
                "diagnostic_values": diagnostic_values}
    if limits:
        declared["limits"] = dict(limits)
    return declared
