"""Ticket #23: handle ownership, bounded batches, close semantics and error records."""
import datetime
import tempfile
import threading
import unittest
from decimal import Decimal
from pathlib import Path

import iresiphi
from iresiphi import build as b

import python_support as support

RECORD_FIELDS = {
    "code", "phase", "message", "context", "severity", "subject", "check",
    "affected_outputs", "affected_count", "count_status", "samples",
    "samples_truncated", "remediation", "values", "report_id",
}


def lines_recipe(columns=None):
    columns = columns or [("id", b.ref("lines", "id")),
                          ("amount", b.ref("lines", "amount")),
                          ("day", b.lit(datetime.date(2026, 11, 12)))]
    return b.recipe("lines", [b.source("lines", {
        "id": b.field("/id", b.string_type()),
        "amount": b.field("/amount", b.decimal_type(20, 2)),
    }, ["id"], array="/lines")], [b.output("lines_out", "lines", [b.column(n, e) for n, e in columns])])


class Handles(unittest.TestCase):
    ROWS = 250

    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory()
        cls.root = Path(cls.directory.name)
        records = ",".join(f'{{"id":"{i:05d}","amount":"{i}.{i % 100:02d}"}}' for i in range(cls.ROWS))
        (cls.root / "lines.json").write_text(f'{{"lines":[{records}]}}', encoding="utf-8")
        cls.bindings = {"lines": b.binding(cls.root / "lines.json", "json")}

    @classmethod
    def tearDownClass(cls):
        cls.directory.cleanup()

    def policy(self, **limits):
        return b.policy([self.root], [self.root], limits=limits or None)

    def run_lines(self, **limits):
        return iresiphi.run(iresiphi.Recipe(lines_recipe()), self.bindings, self.policy(**limits))

    # -- bounded batches ----------------------------------------------------------

    def test_iteration_copies_bounded_batches(self):
        with self.run_lines(batch_rows=100) as result:
            self.assertEqual(result.batch_row_limit, 100)
            sizes = [len(batch) for batch in result.batches("lines_out")]
            self.assertEqual(sizes, [100, 100, 50])
            sizes = [len(batch) for batch in result.batches("lines_out", rows=64)]
            self.assertEqual(sizes, [64, 64, 64, 58])
            with self.assertRaises(iresiphi.IresiphiError) as raised:
                result.batch("lines_out", rows=101)
            self.assertEqual(raised.exception.code, "RESOURCE_LIMIT")
            rows = [row for batch in result.batches("lines_out") for row in batch]
            self.assertEqual(len(rows), self.ROWS)
            self.assertEqual(sorted(r["id"] for r in rows), [f"{i:05d}" for i in range(self.ROWS)])
            by_id = {r["id"]: r for r in rows}
            self.assertEqual(by_id["00123"]["amount"], Decimal("123.23"))
            self.assertEqual(str(by_id["00007"]["amount"]), "7.07")
            self.assertEqual(by_id["00007"]["day"], datetime.date(2026, 11, 12))
            self.assertEqual(list(by_id["00007"]), ["id", "amount", "day", "_status", "_reason"])

    def test_batch_row_cap_is_8192_whatever_the_policy(self):
        with self.run_lines(batch_rows=100_000) as result:
            self.assertEqual(result.batch_row_limit, iresiphi.MAX_BATCH_ROWS)
            self.assertEqual(iresiphi.MAX_BATCH_ROWS, 8192)
            with self.assertRaises(iresiphi.IresiphiError) as raised:
                result.batch("lines_out", rows=8193)
            self.assertEqual(raised.exception.code, "RESOURCE_LIMIT")
            self.assertEqual(len(result.batch("lines_out", rows=8192)), self.ROWS)

    def test_byte_budget_splits_batches_and_rejects_oversized_records(self):
        # The policy's per-batch decoded-byte budget also bounds Python copies:
        # a batch stops before it would exceed the budget. A wide output column
        # makes each copied row (~1.1 KB) larger than its decoded source record.
        wide = lines_recipe([("id", b.ref("lines", "id")), ("note", b.lit("x" * 1000))])
        policy = self.policy(batch_rows=5, batch_bytes=3000)
        with iresiphi.run(iresiphi.Recipe(wide), self.bindings, policy) as result:
            sizes = [len(batch) for batch in result.batches("lines_out")]
            self.assertEqual(sum(sizes), self.ROWS)
            self.assertEqual(set(sizes), {2})
        # A single record larger than the budget is rejected, never truncated.
        policy = self.policy(batch_rows=1, batch_bytes=800)
        with iresiphi.run(iresiphi.Recipe(wide), self.bindings, policy) as result:
            with self.assertRaises(iresiphi.IresiphiError) as raised:
                result.batch("lines_out")
            self.assertEqual(raised.exception.code, "RESOURCE_LIMIT")

    def test_offsets_and_unknown_tables(self):
        with self.run_lines() as result:
            self.assertEqual(result.row_count("lines_out"), self.ROWS)
            self.assertEqual(result.batch("lines_out", offset=self.ROWS), [])
            self.assertEqual(len(result.batch("lines_out", offset=self.ROWS - 3)), 3)
            with self.assertRaises(iresiphi.IresiphiError) as raised:
                result.batch("nope")
            self.assertEqual(raised.exception.code, "UNKNOWN_OUTPUT")
            evidence = [r for batch in result.evidence_batches("primary", rows=100) for r in batch]
            self.assertEqual(len(evidence), result.evidence_count("primary"))

    # -- close semantics --------------------------------------------------------------

    def test_context_manager_repeated_close_access_after_close_and_retained_values(self):
        with self.run_lines() as result:
            rows = result.batch("lines_out", rows=5)
            manifest = result.manifest
            diagnostics = result.diagnostics
            schema = result.schema("lines_out")
            iterator = result.batches("lines_out", rows=10)
            first = next(iterator)
            run_id = result.run_id
        self.assertTrue(result.closed)
        result.close()
        result.close()
        accessors = {
            "status": lambda: result.status,
            "run_id": lambda: result.run_id,
            "manifest": lambda: result.manifest,
            "diagnostics": lambda: result.diagnostics,
            "outputs": lambda: result.outputs,
            "schema": lambda: result.schema("lines_out"),
            "row_count": lambda: result.row_count("lines_out"),
            "batch": lambda: result.batch("lines_out"),
            "batches": lambda: result.batches("lines_out"),
            "evidence": lambda: result.evidence(),
            "evidence_count": lambda: result.evidence_count(),
            "iterator": lambda: next(iterator),
        }
        for name, access in accessors.items():
            with self.assertRaises(iresiphi.HandleClosedError, msg=name) as raised:
                access()
            self.assertEqual(raised.exception.code, "HANDLE_CLOSED", name)
            self.assertIsInstance(raised.exception, iresiphi.IresiphiError)
        # Values returned before close own their data.
        self.assertEqual(len(rows), 5)
        self.assertIsInstance(rows[0]["amount"], Decimal)
        self.assertEqual(len(first), 10)
        self.assertEqual(manifest["run_id"], run_id)
        self.assertEqual(diagnostics, [])
        self.assertEqual({k: schema[1][k] for k in ("name", "type")}, {"name": "amount", "type": {"kind": "decimal", "precision": 20, "scale": 2}})
        self.assertEqual(schema[1]["ownership"], "owned")
        self.assertEqual(repr(result), "<iresiphi.RunResult closed>")

    def test_recipe_and_catalog_close(self):
        recipe = iresiphi.Recipe(lines_recipe())
        text = recipe.to_json()
        digest = recipe.hash
        with recipe:
            self.assertFalse(recipe.closed)
        recipe.close()
        recipe.close()
        for name, access in {
            "to_json": recipe.to_json, "canonical_json": recipe.canonical_json,
            "hash": lambda: recipe.hash, "validate": recipe.validate, "explain": recipe.explain,
            "copy": recipe.copy, "to_dict": recipe.to_dict,
            "run": lambda: iresiphi.run(recipe, self.bindings, self.policy()),
        }.items():
            with self.assertRaises(iresiphi.HandleClosedError, msg=name):
                access()
        self.assertEqual(iresiphi.Recipe.from_json(text).hash, digest)
        catalog = iresiphi.Catalog({"name": "shared", "sources": [lines_recipe()["sources"][0]]})
        target = iresiphi.Recipe(lines_recipe())
        catalog.close()
        catalog.close()
        with self.assertRaises(iresiphi.HandleClosedError):
            target.bundle_from(catalog, "lines")

    def test_catalog_bundling(self):
        declaration = lines_recipe()
        shared = dict(declaration["sources"][0], id="shared_lines")
        with iresiphi.Catalog({"name": "team", "revision": "7", "sources": [shared]}) as catalog:
            recipe = iresiphi.Recipe(declaration)
            recipe.bundle_from(catalog, "shared_lines")
            bundled = recipe.to_dict()["bundled_definitions"]["shared_lines"]
            self.assertIn("team", bundled["provenance"])
            reopened = iresiphi.Recipe.from_json(recipe.to_json())
            self.assertEqual(reopened.hash, recipe.hash)
            with self.assertRaises(iresiphi.IresiphiError) as raised:
                recipe.bundle_from(catalog, "missing")
            self.assertEqual(raised.exception.code, "MISSING_REFERENCE")

    # -- ownership ------------------------------------------------------------------

    def test_recipe_owns_a_copy_of_its_declaration(self):
        declaration = lines_recipe()
        recipe = iresiphi.Recipe(declaration)
        digest = recipe.hash
        declaration["outputs"][0]["columns"].clear()
        declaration["name"] = "mutated"
        self.assertEqual(recipe.hash, digest)
        copy = recipe.copy()
        recipe.close()
        self.assertEqual(copy.hash, digest)

    def test_closing_inputs_during_a_run_does_not_affect_it(self):
        recipe = iresiphi.Recipe(lines_recipe())
        bindings = {"lines": dict(self.bindings["lines"])}
        outcome = {}

        def work():
            try:
                outcome["result"] = iresiphi.run(recipe, bindings, self.policy())
            except BaseException as error:  # pragma: no cover - reported below
                outcome["error"] = error

        worker = threading.Thread(target=work)
        worker.start()
        recipe.close()
        bindings.clear()
        worker.join()
        if "error" in outcome:
            # The run may not have copied the recipe before close; then it must
            # fail cleanly with HANDLE_CLOSED, never crash or read freed state.
            self.assertEqual(outcome["error"].code, "HANDLE_CLOSED")
        else:
            with outcome["result"] as result:
                self.assertEqual(result.row_count("lines_out"), self.ROWS)

    # -- portability and error records ------------------------------------------------

    def test_host_only_values_are_rejected_as_non_portable(self):
        declaration = lines_recipe()
        cases = {
            "callable": lambda d: d["outputs"][0]["columns"][0].update(expr=lambda row: row),
            "float": lambda d: d["outputs"][0]["columns"][0].update(
                expr={"op": "literal", "value": {"kind": "decimal", "value": 1.5, "precision": 2, "scale": 1}}),
            "bare decimal": lambda d: d["outputs"][0]["columns"][0].update(expr=Decimal("1.5")),
            "object": lambda d: d.update(annotations={"x": object()}),
        }
        for name, mutate in cases.items():
            candidate = lines_recipe()
            mutate(candidate)
            with self.assertRaises(iresiphi.IresiphiError, msg=name) as raised:
                iresiphi.Recipe(candidate)
            self.assertEqual(raised.exception.code, "NON_PORTABLE", name)
            self.assertEqual(raised.exception.phase, "portability", name)
        for value in [1.5, float("nan"), print, datetime.datetime(2026, 1, 1)]:
            with self.assertRaises(iresiphi.IresiphiError) as raised:
                b.lit(value)
            self.assertEqual(raised.exception.code, "NON_PORTABLE")
        for op in ["python", "callback", "udf"]:
            candidate = lines_recipe([("x", {"op": op, "function": "f"})])
            with self.assertRaises(iresiphi.IresiphiError) as raised:
                iresiphi.Recipe(candidate)
            self.assertEqual(raised.exception.code, "UNSUPPORTED_OPERATION", op)
        bindings = {"lines": {"path": self.root / "lines.json", "adapter": "json",
                              "settings": {"hook": lambda: None}}}
        with self.assertRaises(iresiphi.IresiphiError) as raised:
            iresiphi.run(iresiphi.Recipe(declaration), bindings, self.policy())
        self.assertEqual(raised.exception.code, "NON_PORTABLE")

    def test_errors_carry_the_full_diagnostic_record(self):
        with self.assertRaises(iresiphi.IresiphiError) as raised:
            iresiphi.Recipe.from_json('{"format_version":"0.1","format_version":"0.1"}')
        self.assertEqual(raised.exception.code, "DUPLICATE_KEY")
        with self.assertRaises(iresiphi.IresiphiError) as raised:
            iresiphi.Recipe(dict(lines_recipe(), format_version="9.9"))
        self.assertEqual(raised.exception.code, "UNSUPPORTED_VERSION")
        (self.root / "elsewhere").mkdir(exist_ok=True)
        policy = b.policy([self.root / "elsewhere"], [])
        with self.assertRaises(iresiphi.IresiphiError) as raised:
            iresiphi.run(iresiphi.Recipe(lines_recipe()), self.bindings, policy)
        error = raised.exception
        self.assertEqual(error.code, "POLICY_DENIED")
        self.assertTrue(RECORD_FIELDS <= set(error.record), set(error.record) ^ RECORD_FIELDS)
        self.assertEqual(error.record["code"], error.code)
        self.assertEqual(error.report_id, error.report.run_id)
        self.assertEqual(error.record["report_id"], error.report_id)
        self.assertEqual(error.report.status, "failed")
        self.assertFalse(error.report.manifest["complete"])
        self.assertEqual(error.report.diagnostics[0]["code"], "POLICY_DENIED")
        self.assertIsNotNone(error.remediation)
        self.assertTrue(str(error).startswith("POLICY_DENIED ["))
        with self.assertRaises(iresiphi.IresiphiError) as raised:
            iresiphi.run(iresiphi.Recipe(lines_recipe()), {"lines": {"path": "x"}}, self.policy())
        self.assertEqual(raised.exception.code, "INVALID_BINDING")
        with self.assertRaises(iresiphi.IresiphiError) as raised:
            iresiphi.run(iresiphi.Recipe(lines_recipe()), self.bindings, {"allowed_read_roots": "x"})
        self.assertEqual(raised.exception.code, "INVALID_POLICY")
        with self.assertRaises(iresiphi.IresiphiError) as raised:
            iresiphi.Recipe(lines_recipe()).validate(self.bindings, None)
        self.assertEqual(raised.exception.code, "INVALID_ARGUMENT")


if __name__ == "__main__":
    unittest.main()
