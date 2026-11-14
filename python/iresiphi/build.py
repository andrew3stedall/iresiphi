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


def relationship(id, from_, to, from_keys, to_keys, cardinality, validity=None, strict=False,
                 period=None):
    """A keyed relationship. ``period`` (see ``period_match``) adds a derived period key
    to a relationship from a population."""
    declared = {"id": id, "from": from_, "to": to, "from_keys": list(from_keys),
                "to_keys": list(to_keys), "cardinality": cardinality}
    if validity is not None:
        declared["validity"] = validity
    if strict:
        declared["strict"] = True
    if period is not None:
        declared["period"] = period
    return declared


def months(field, values=None, from_=None):
    """A finite period set of calendar months (``YYYY-MM``), named ``field`` on each member.

    Give either the listed ``values`` or ``from_=(source, field)``: the distinct values
    of a named field of a named source. Nothing infers the period set."""
    declared = {"field": field, "calendar": "month"}
    if values is not None:
        declared["values"] = list(values)
    if from_ is not None:
        source_id, source_field = from_
        declared["from"] = {"source": source_id, "field": source_field}
    return declared


def population(id, recipients, period, filter=None):
    """An explicit population: records of ``recipients`` that pass ``filter``, crossed
    with the ``period`` set (see ``months``). Outputs anchor at ``id``."""
    declared = {"id": id, "recipients": recipients, "period": period}
    if filter is not None:
        declared["filter"] = filter
    return declared


def period_match(key, date):
    """The member's period field ``key`` equals the calendar month of the target's
    ``date``. Dates are business-local; nothing converts time zones."""
    return {"key": key, "date": date, "calendar": "month"}


def allocation(name, quantity, partition, via, weight, precision, scale, residual,
               recipient_filter=None, null_weight=None, unallocated=None):
    """Allocate ``quantity=(source, field)`` onto an output's population members.

    ``partition`` is the quantity source's grain; ``via`` a many_to_one relationship
    from the population to that source; ``residual`` is ``"largest_remainder"`` or
    ``"report"``. ``recipient_filter`` runs before normalization. ``null_weight="zero"``
    and ``unallocated="report"`` are the declared alternatives to an error."""
    source_id, field_name = quantity
    declared = {"name": name, "quantity": {"source": source_id, "field": field_name},
                "partition": list(partition), "via": via, "weight": weight,
                "precision": precision, "scale": scale, "residual": residual}
    if recipient_filter is not None:
        declared["recipient_filter"] = recipient_filter
    if null_weight is not None:
        declared["null_weight"] = null_weight
    if unallocated is not None:
        declared["unallocated"] = unallocated
    return declared


def feature_declaration(id, expr):
    return {"id": id, "expr": expr}


def column(name, expr):
    return {"name": name, "expr": expr}


def output(id, anchor, columns, eligibility=None, filter=None, detail=False,
           group_by=(), aggregates=(), complete=(), allocations=()):
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
    if complete:
        declared["complete"] = list(complete)
    if allocations:
        declared["allocations"] = list(allocations)
    return declared


def recipe(name, sources, outputs, relationships=(), features=(),
           required_capabilities=(), annotations=None, populations=()):
    declared = {"format_version": FORMAT_VERSION, "name": name, "sources": list(sources),
                "relationships": list(relationships), "features": list(features),
                "outputs": list(outputs)}
    if populations:
        declared["populations"] = list(populations)
    if required_capabilities:
        declared["required_capabilities"] = list(required_capabilities)
    if annotations:
        declared["annotations"] = dict(annotations)
    return declared


# -- runtime inputs ------------------------------------------------------------------

def binding(path, adapter, **settings):
    """Bind one source to a local file at run time (never saved in the recipe)."""
    return {"path": path, "adapter": adapter, "settings": settings}


def policy(read_roots, output_roots=(), limits=None, diagnostic_values=False, placement=None,
           transfers=None, evidence=None):
    """`placement` is "local" (the default), "prefer_source" or "require_source".

    `transfers` is "none" (the default) or "keys": whether lookup key values may
    be sent to a lookup target's source to reduce what it returns.

    `evidence` is "problems" (the default) or "full": whether a run records
    evidence only for lines that did not resolve, or for every line."""
    declared = {"allowed_read_roots": list(read_roots),
                "allowed_output_roots": list(output_roots),
                "diagnostic_values": diagnostic_values}
    if placement is not None:
        declared["placement"] = placement
    if transfers is not None:
        declared["transfers"] = transfers
    if evidence is not None:
        declared["evidence"] = evidence
    if limits:
        declared["limits"] = dict(limits)
    return declared
