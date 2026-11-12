"""Ticket #23 release gates through the installed Python package.

Run against an installed wheel from the repository root:
    python -m unittest discover -s tests -p "python_*.py" -v

Rust/Python parity uses tests/fixtures/two_suppliers.parity.json, which
tests/python_parity.rs checks (and regenerates) through the Rust API.
"""
import datetime
import json
import tempfile
import unittest
from decimal import Decimal
from pathlib import Path

import iresiphi
from iresiphi import build as b

import python_support as support


def tagged_rows(result, table):
    """Python rows converted back to tagged text using the declared schema."""
    schema = {c["name"]: c["type"] for c in result.schema(table)}
    rows = []
    for batch in result.batches(table):
        for row in batch:
            converted = {}
            for name, value in row.items():
                kind = schema.get(name) or {}
                if value is None:
                    converted[name] = {"kind": "null"}
                elif kind.get("kind") == "decimal":
                    converted[name] = b.tagged(value, kind["precision"], kind["scale"])
                else:
                    converted[name] = b.tagged(value)
            rows.append(converted)
    return sorted(rows, key=lambda r: json.dumps(r, sort_keys=True))


def expected_rows(golden_rows):
    return sorted(golden_rows, key=lambda r: json.dumps(r, sort_keys=True))


class TwoSupplierParity(unittest.TestCase):
    """Rust/Python public expected-result and error parity on the agreed fixture."""

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        support.write_inputs(self.root)
        self.golden = json.loads(support.GOLDEN.read_text(encoding="utf-8"), parse_float=Decimal)
        self.recipe = iresiphi.Recipe(support.two_supplier_declaration())

    def tearDown(self):
        self.recipe.close()
        self.directory.cleanup()

    def test_python_builders_produce_the_rust_recipe(self):
        self.assertEqual(self.recipe.hash, self.golden["recipe_hash"])
        self.assertEqual(self.recipe.canonical_json(), self.golden["canonical_json"])

    def test_expected_results_match_rust(self):
        with iresiphi.run(self.recipe, support.bindings(self.root), support.policy(self.root)) as result:
            expected = self.golden["expected"]
            self.assertEqual(result.status, "succeeded_with_unresolved")
            self.assertEqual(result.status, expected["status"])
            manifest = result.manifest
            combined = manifest["totals"]["combined"]["AUD"]
            self.assertEqual(combined["columns"]["billed"]["population"], "180.00")
            self.assertEqual(combined["columns"]["billed"]["comparable"], "160.00")
            self.assertEqual(combined["columns"]["expected"]["comparable"], "145.00")
            self.assertEqual(combined["columns"]["variance"]["comparable"], "15.00")
            self.assertEqual(combined["columns"]["billed"]["unresolved"], "20.00")
            self.assertEqual(combined["rows"], {"population": 3, "resolved": 2, "unresolved": 1})
            self.assertEqual(manifest["totals"], expected["totals"])
            self.assertEqual(manifest["reconciliation"], expected["reconciliation"])
            self.assertEqual(sorted(result.outputs), sorted(expected["tables"]))
            for table, rows in expected["tables"].items():
                self.assertEqual(result.schema(table), expected["schema"][table]["columns"])
                self.assertEqual(tagged_rows(result, table), expected_rows(rows), table)
            warnings = sorted(
                ({"code": d["code"], "severity": d["severity"], "subject": d["subject"],
                  "affected_count": d["affected_count"]} for d in result.diagnostics),
                key=lambda w: json.dumps(w, sort_keys=True))
            self.assertEqual(warnings, expected["diagnostics"])
            for kind, count in expected["evidence_counts"].items():
                self.assertEqual(result.evidence_count(kind), count)
            # Exact native values for the comparable line.
            a = {row["id"]: row for row in result.batch("a_review")}
            self.assertEqual(a["1"]["billed"], Decimal("110.00"))
            self.assertEqual(str(a["1"]["expected"]), "100.00")
            self.assertEqual(a["1"]["variance"], Decimal("10.00"))
            self.assertIsNone(a["2"]["expected"])
            self.assertEqual(a["2"]["_status"], "unmatched")

    def assertOutcome(self, error, expected):
        self.assertEqual(error.code, expected["code"])
        self.assertEqual(error.phase, expected["phase"])
        self.assertEqual(error.check, expected["check"])
        self.assertEqual(error.subject, expected["subject"])
        self.assertEqual(error.report.status, expected["status"])
        self.assertEqual(error.report_id, error.report.run_id)

    def test_error_parity_missing_binding(self):
        bindings = support.bindings(self.root)
        del bindings["rates"]
        with self.assertRaises(iresiphi.IresiphiError) as raised:
            iresiphi.run(self.recipe, bindings, support.policy(self.root))
        self.assertOutcome(raised.exception, self.golden["errors"]["missing_binding"])

    def test_error_parity_duplicate_identity(self):
        (self.root / "mapping.csv").write_text(
            "id,supplier,raw,item\nm1,A,service A,shared\nm1,B,service B,shared\n", encoding="utf-8")
        with self.assertRaises(iresiphi.IresiphiError) as raised:
            iresiphi.run(self.recipe, support.bindings(self.root), support.policy(self.root))
        self.assertOutcome(raised.exception, self.golden["errors"]["duplicate_identity"])
        self.assertEqual(raised.exception.record["count_status"], "exact")

    def test_error_parity_static_division(self):
        declaration = support.two_supplier_declaration()
        a_review = next(o for o in declaration["outputs"] if o["id"] == "a_review")
        variance = next(c for c in a_review["columns"] if c["name"] == "variance")
        variance["expr"] = {"op": "divide", "left": b.ref("a", "billed"), "right": b.ref("a", "quantity")}
        expected = self.golden["errors"]["division"]
        self.assertEqual(expected["stage"], "load")
        with self.assertRaises(iresiphi.IresiphiError) as raised:
            iresiphi.Recipe(declaration)
        self.assertEqual(raised.exception.code, expected["code"])
        self.assertEqual(raised.exception.phase, expected["phase"])
        self.assertIsNone(raised.exception.report_id)


class SaveReopenRebind(unittest.TestCase):
    def test_save_reopen_and_rebind_to_a_second_input_set(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first, second = root / "first", root / "second"
            first.mkdir()
            second.mkdir()
            support.write_inputs(first)
            support.write_inputs(second, b_json=support.B_JSON.replace('"charge":"50"', '"charge":"60"'))
            recipe = iresiphi.Recipe(support.two_supplier_declaration())
            path = root / "review.iresiphi"
            iresiphi.save(recipe, path)
            saved = path.read_text(encoding="utf-8")
            self.assertEqual(saved, recipe.to_json())
            # Secret-free and machine-path-free: no binding data is saved.
            self.assertNotIn(str(root), saved)
            with self.assertRaises(FileExistsError):
                iresiphi.save(recipe, path)
            reopened = iresiphi.load(path)
            self.assertEqual(reopened.hash, recipe.hash)
            self.assertEqual(reopened.canonical_json(), recipe.canonical_json())
            self.assertEqual(iresiphi.Recipe.from_json(reopened.to_json()).hash, recipe.hash)
            self.assertEqual(iresiphi.Recipe(reopened.to_dict()).hash, recipe.hash)
            # Missing bindings only fail at bound validation/run, not on reopen.
            self.assertTrue(reopened.validate()["valid"])
            report = reopened.validate({}, support.policy(first))
            self.assertFalse(report["valid"])
            self.assertIn("MISSING_BINDING", json.dumps(report))
            full = reopened.validate(support.bindings(first), support.policy(first), "full")
            self.assertTrue(full["valid"], full)
            explanation = reopened.explain(support.bindings(first), support.policy(first))
            self.assertTrue(explanation["bound"]["feasible"])
            self.assertIn("nodes", reopened.explain())
            totals = {}
            for name, folder in [("first", first), ("second", second)]:
                with iresiphi.run(reopened, support.bindings(folder), support.policy(folder)) as result:
                    totals[name] = result.manifest["totals"]["combined"]["AUD"]["columns"]
                    self.assertEqual(result.manifest["recipe_hash"], recipe.hash)
            self.assertEqual(totals["first"]["billed"]["population"], "180.00")
            self.assertEqual(totals["second"]["billed"]["population"], "190.00")
            self.assertEqual(totals["second"]["billed"]["comparable"], "170.00")
            self.assertEqual(totals["second"]["variance"]["comparable"], "25.00")

    def test_bundle_delivery_and_read_back(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            support.write_inputs(root)
            recipe = iresiphi.Recipe(support.two_supplier_declaration())
            destination = root / "bundle"
            with iresiphi.run(recipe, support.bindings(root), support.policy(root),
                              destination=destination) as result:
                self.assertEqual(result.status, "succeeded_with_unresolved")
                self.assertTrue(result.manifest["delivery"]["committed"])
            tables = iresiphi.read_bundle(destination)
            billed = sorted(row["billed"] for row in tables["a_review"])
            self.assertEqual(billed, [Decimal("20.00"), Decimal("110.00")])
            with self.assertRaises(iresiphi.IresiphiError) as raised:
                iresiphi.run(recipe, support.bindings(root), support.policy(root), destination=destination)
            self.assertEqual(raised.exception.code, "DESTINATION_EXISTS")
            self.assertEqual(raised.exception.report.status, "failed")


def without_references(value):
    """The golden's form of manifest sections: source references (whose binding
    identity depends on file paths) removed, as tests/customer_month_parity.rs does."""
    if isinstance(value, dict):
        return {k: without_references(v) for k, v in value.items()
                if k not in ("quantity_reference", "unmatched_samples")}
    if isinstance(value, list):
        return [without_references(v) for v in value]
    return value


class CustomerMonthParity(unittest.TestCase):
    """Slice 2 (#28, #29): explicit population, completeness and allocation on the
    customer/month fixture, compared to tests/fixtures/customer_month.parity.json."""

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        support.write_customer_month(self.root)
        self.golden = json.loads(support.CUSTOMER_MONTH_GOLDEN.read_text(encoding="utf-8"),
                                 parse_float=Decimal)
        self.bindings = support.customer_month_bindings(self.root)
        self.policy = support.policy(self.root)

    def tearDown(self):
        self.directory.cleanup()

    def assertMatchesGolden(self, result, expected):
        self.assertEqual(result.status, expected["status"])
        manifest = result.manifest
        for key in ["totals", "reconciliation", "allocations"]:
            self.assertEqual(manifest[key], expected[key], key)
        self.assertEqual(without_references(manifest["populations"]), expected["populations"])
        self.assertEqual(sorted(result.outputs), sorted(expected["tables"]))
        for table, rows in expected["tables"].items():
            self.assertEqual(result.schema(table), expected["schema"][table]["columns"])
            self.assertEqual(tagged_rows(result, table), expected_rows(rows), table)
        diagnostics = sorted(
            ({"code": d["code"], "severity": d["severity"], "subject": d["subject"],
              "affected_count": d["affected_count"]} for d in result.diagnostics),
            key=lambda d: json.dumps(d, sort_keys=True))
        self.assertEqual(diagnostics, expected["diagnostics"])
        for kind, count in expected["evidence_counts"].items():
            self.assertEqual(result.evidence_count(kind), count, kind)
        self.assertEqual(without_references(result.evidence("allocation")),
                         expected["allocation_evidence"])

    def test_python_builders_produce_the_rust_recipe(self):
        recipe = iresiphi.Recipe(support.customer_month_declaration())
        self.assertEqual(recipe.hash, self.golden["recipe_hash"])
        self.assertEqual(recipe.canonical_json(), self.golden["canonical_json"])
        self.assertEqual(iresiphi.Recipe.from_json(recipe.to_json()).hash, recipe.hash)

    def test_expected_results_match_rust(self):
        recipe = iresiphi.Recipe(support.customer_month_declaration())
        with iresiphi.run(recipe, self.bindings, self.policy) as result:
            self.assertMatchesGolden(result, self.golden["expected"])
            rows = {(r["CustNo"], r["month"]): r for r in result.batch("customer_targets")}
            self.assertEqual(len(rows), 6)
            allocated = {key: row["allocated_target"] for key, row in rows.items()}
            self.assertEqual(allocated, {
                ("A", "2026-01"): Decimal("225.00"), ("B", "2026-01"): Decimal("675.00"),
                ("C", "2026-01"): Decimal("300.00"), ("A", "2026-02"): Decimal("200.00"),
                ("B", "2026-02"): Decimal("600.00"), ("C", "2026-02"): Decimal("400.00")})
            self.assertEqual(rows[("A", "2026-02")]["sales"], Decimal("0.00"))
            self.assertEqual(rows[("B", "2026-02")]["transactions"], 1)
            columns = result.manifest["totals"]["by_output"]["customer_targets"]["AUD"]["columns"]
            self.assertEqual(columns["sales"]["population"], "470.00")
            self.assertNotIn("target_context", columns)
            self.assertEqual(columns["allocated_target"]["allocation"]["source_total"], "2400.00")
            self.assertEqual(sum(row["target_context"] for row in rows.values()), Decimal("4100.00"))

    def test_filter_and_missing_target_scenarios_match_rust(self):
        scenarios = self.golden["scenarios"]
        selected = support.customer_month_declaration(
            output_filter=b.equal(b.ref("customer_targets", "CustNo"), b.lit("A")))
        restricted = support.customer_month_declaration(recipient_filter=b.or_(
            b.equal(b.ref("customer_month", "CustNo"), b.lit("A")),
            b.equal(b.ref("customer_month", "SalesArea"), b.lit("South"))))
        for name, declaration in [("selected_a", selected), ("north_restricted_to_a", restricted)]:
            with iresiphi.run(iresiphi.Recipe(declaration), self.bindings, self.policy) as result:
                self.assertMatchesGolden(result, scenarios[name])
        support.write_customer_month(self.root, targets=support.TARGETS.replace("South,2026-02,400\n", ""))
        recipe = iresiphi.Recipe(support.customer_month_declaration())
        with iresiphi.run(recipe, self.bindings, self.policy) as result:
            self.assertMatchesGolden(result, scenarios["missing_target"])

    def test_error_parity_null_weight(self):
        support.write_customer_month(self.root, customers=support.CUSTOMERS.replace("B,North,3", "B,North,\\N"))
        recipe = iresiphi.Recipe(support.customer_month_declaration(nullable_share=True))
        expected = self.golden["errors"]["null_weight"]
        with self.assertRaises(iresiphi.IresiphiError) as raised:
            iresiphi.run(recipe, self.bindings, self.policy)
        for key in ["code", "phase", "check", "subject"]:
            self.assertEqual(getattr(raised.exception, key), expected[key], key)
        self.assertEqual(raised.exception.report.status, expected["status"])

    def test_bundle_carries_allocation_evidence(self):
        recipe = iresiphi.Recipe(support.customer_month_declaration())
        destination = self.root / "bundle"
        with iresiphi.run(recipe, self.bindings, self.policy, destination=destination) as result:
            self.assertEqual(result.status, "succeeded")
        tables = iresiphi.read_bundle(destination)
        self.assertEqual(len(tables["customer_targets"]), 6)
        self.assertEqual(len(tables["_allocation_evidence"]), 4)


def literal_recipe(columns, name="python exact values"):
    return b.recipe(name, [b.source("lines", {"id": b.field("/id", b.string_type())}, ["id"], array="/lines")],
                    [b.output("values", "lines", [b.column(n, e) for n, e in columns])])


class ExactValues(unittest.TestCase):
    """Values beyond 2^53, Int64 bounds, decimal ties and scale 38, null and date
    boundaries round-trip unchanged between Rust and Python."""

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        (self.root / "lines.json").write_text('{"lines":[{"id":"a"}]}', encoding="utf-8")
        self.bindings = {"lines": b.binding(self.root / "lines.json", "json")}
        self.policy = b.policy([self.root], [self.root])

    def tearDown(self):
        self.directory.cleanup()

    def run_row(self, columns):
        recipe = iresiphi.Recipe(literal_recipe(columns))
        reopened = iresiphi.Recipe.from_json(recipe.to_json())
        self.assertEqual(reopened.hash, recipe.hash)
        with iresiphi.run(reopened, self.bindings, self.policy) as result:
            self.assertEqual(result.status, "succeeded")
            schema = {c["name"]: c["type"] for c in result.schema("values")}
            [row] = result.batch("values")
        return row, schema, recipe

    def test_python_values_round_trip_through_rust(self):
        values = {
            "beyond_2_53": 2**53 + 1,
            "negative_beyond_2_53": -(2**53 + 1),
            "int64_max": 2**63 - 1,
            "int64_min": -(2**63),
            "zero": 0,
            "decimal_beyond_2_53": Decimal("9007199254740993.01"),
            "scale_38": Decimal("0.12345678901234567890123456789012345678"),
            "negative_scale_38": Decimal("-0.00000000000000000000000000000000000001"),
            "precision_38": Decimal("99999999999999999999999999999999999999"),
            "negative_precision_38": Decimal("-99999999999999999999999999999999999999"),
            "trailing_zero": Decimal("1.10"),
            "date_min": datetime.date(1, 1, 1),
            "date_max": datetime.date(9999, 12, 31),
            "leap_day": datetime.date(2024, 2, 29),
            "empty_string": "",
            "escaped_null_text": "\\N",
            "unicode": "Grüße ✓",
            "true": True,
            "false": False,
        }
        row, schema, recipe = self.run_row([(name, b.lit(value)) for name, value in values.items()])
        for name, value in values.items():
            self.assertEqual(row[name], value, name)
            self.assertIs(type(row[name]), type(value), name)
            if isinstance(value, Decimal):
                self.assertEqual(str(row[name]), str(value), name)
                self.assertEqual(schema[name]["scale"], max(0, -value.as_tuple().exponent), name)
        self.assertEqual(schema["scale_38"], {"kind": "decimal", "precision": 38, "scale": 38})
        self.assertEqual(schema["precision_38"], {"kind": "decimal", "precision": 38, "scale": 0})
        self.assertEqual(schema["int64_max"], {"kind": "int64"})
        saved = recipe.to_json()
        self.assertIn('"9223372036854775807"', saved)
        self.assertIn('"0.12345678901234567890123456789012345678"', saved)

    def test_declared_precision_and_scale_survive(self):
        row, schema, _ = self.run_row([("money", b.lit(Decimal("7.5"), 18, 2))])
        self.assertEqual(str(row["money"]), "7.50")
        self.assertEqual(schema["money"], {"kind": "decimal", "precision": 18, "scale": 2})

    def test_quantization_ties_and_nulls(self):
        cases = {
            "half_even_down": (Decimal("1.005"), "half_even", "1.00"),
            "half_even_up": (Decimal("1.015"), "half_even", "1.02"),
            "half_even_negative": (Decimal("-1.005"), "half_even", "-1.00"),
            "half_even_negative_up": (Decimal("-1.015"), "half_even", "-1.02"),
            "toward_zero": (Decimal("1.019"), "toward_zero", "1.01"),
            "toward_zero_negative": (Decimal("-1.019"), "toward_zero", "-1.01"),
        }
        columns = [(name, b.quantize(b.lit(value), 10, 2, rounding))
                   for name, (value, rounding, _) in cases.items()]
        columns += [("null_int", b.cast(b.lit(None), b.int64_type())),
                    ("null_decimal", b.cast(b.lit(None), b.decimal_type(38, 38))),
                    ("null_date", b.cast(b.lit(None), b.date_type())),
                    ("null_equals_null", b.equal(b.cast(b.lit(None), b.int64_type()),
                                                 b.cast(b.lit(None), b.int64_type())))]
        row, _, _ = self.run_row(columns)
        for name, (_, _, expected) in cases.items():
            self.assertEqual(str(row[name]), expected, name)
        for name in ["null_int", "null_decimal", "null_date", "null_equals_null"]:
            self.assertIsNone(row[name], name)

    def test_int64_overflow_fails_the_run(self):
        recipe = iresiphi.Recipe(literal_recipe([("over", b.add(b.lit(2**63 - 1), b.lit(1)))]))
        with self.assertRaises(iresiphi.IresiphiError) as raised:
            iresiphi.run(recipe, self.bindings, self.policy)
        self.assertEqual(raised.exception.code, "ARITHMETIC_OVERFLOW")
        self.assertEqual(raised.exception.report.status, "failed")

    def test_out_of_range_literals_are_rejected_by_rust(self):
        for value in [2**63, -(2**63) - 1]:
            with self.assertRaises(iresiphi.IresiphiError) as raised:
                iresiphi.Recipe(literal_recipe([("bad", b.lit(value))]))
            self.assertEqual(raised.exception.phase, "validation")
        with self.assertRaises(iresiphi.IresiphiError):
            iresiphi.Recipe(literal_recipe([("bad", b.lit(Decimal("1" * 39)))]))
        with self.assertRaises(iresiphi.IresiphiError):
            iresiphi.Recipe(literal_recipe([("bad", b.lit(Decimal("1.234"), 10, 2))]))

    def test_from_tagged_is_exact(self):
        self.assertEqual(iresiphi.from_tagged({"kind": "int64", "value": "-9223372036854775808"}), -(2**63))
        value = iresiphi.from_tagged({"kind": "decimal", "value": "+007.5", "precision": 5, "scale": 2})
        self.assertEqual(str(value), "7.50")
        self.assertEqual(iresiphi.from_tagged({"kind": "date", "value": "9999-12-31"}),
                         datetime.date(9999, 12, 31))
        self.assertIsNone(iresiphi.from_tagged({"kind": "null"}))
        with self.assertRaises(iresiphi.IresiphiError) as raised:
            iresiphi.from_tagged({"kind": "date", "value": "0000-01-01"})
        self.assertIn(raised.exception.code, {"VALUE_OUT_OF_RANGE", "INVALID_VALUE"})


if __name__ == "__main__":
    unittest.main()
