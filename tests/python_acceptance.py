"""Run against an installed wheel: python tests/python_acceptance.py."""
import datetime
from decimal import Decimal
import json
from pathlib import Path
import tempfile
import unittest
import iresiphi

class PublicPythonAcceptance(unittest.TestCase):
    def test_owned_exact_result_reopen_and_cancel(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "lines.json"
            path.write_text('[{"id":"a"}]')
            recipe = iresiphi.Recipe({
                "format_version":"0.1", "name":"python exact values",
                "sources":[{"id":"lines","identity":["id"],"grain":["id"],
                    "projection":{"fields":{"id":{"path":"/id","data_type":{"kind":"string"}}}}}],
                "outputs":[{"id":"review","anchor":"lines","columns":[
                    {"name":"amount","expr":{"op":"literal","value":{"kind":"decimal","value":"9007199254740993.01","precision":20,"scale":2}}},
                    {"name":"date","expr":{"op":"literal","value":{"kind":"date","value":"2026-11-12"}}},
                    {"name":"integer","expr":{"op":"literal","value":{"kind":"int64","value":"9007199254740993"}}}
                ]}]
            })
            self.assertTrue(recipe.validate()["valid"])
            self.assertIn("nodes",recipe.explain())
            reopened = iresiphi.Recipe.from_json(recipe.to_json())
            recipe.close()
            bindings = {"lines":{"path":str(path),"adapter":"json","settings":{}}}
            policy = {"allowed_read_roots":[root],"allowed_output_roots":[root],"limits":{},"diagnostic_values":False}
            context = iresiphi.ExecutionContext()
            result = context.run(reopened,bindings,policy)
            self.assertEqual(result.status,"succeeded",result.metadata)
            row = result.batch("review",count=1)[0]
            self.assertEqual(row["amount"],Decimal("9007199254740993.01"))
            self.assertIsInstance(row["amount"],Decimal)
            self.assertEqual(row["integer"],9007199254740993)
            self.assertEqual(row["date"],datetime.date(2026,11,12))
            result.close()
            with self.assertRaisesRegex(RuntimeError,"HANDLE_CLOSED"):
                result.batch("review")
            self.assertEqual(row["amount"],Decimal("9007199254740993.01"))
            token = iresiphi.CancellationToken()
            token.cancel()
            cancelled = context.run(reopened,bindings,policy,cancel=token)
            self.assertEqual(cancelled.status,"cancelled")

if __name__ == "__main__":
    unittest.main()
