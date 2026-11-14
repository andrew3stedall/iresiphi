"""Shared helpers for the Python acceptance suites (no tests here).

The two-supplier fixture mirrors tests/support/invoice.rs: the same input files
and the same recipe, built here with ``iresiphi.build`` instead of Rust.
"""
import importlib.util
import json
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile

from iresiphi import build as b

REPO = Path(__file__).resolve().parent.parent
GOLDEN = REPO / "tests" / "fixtures" / "two_suppliers.parity.json"

A_JSON = '{"supplier":"A","invoice":"A1","date":"2026-11-12","currency":"AUD","lines":[{"id":"1","item":"service A","quantity":"2","billed":"110"},{"id":"2","item":"unknown","quantity":"1","billed":"20"}]}'
B_JSON = '{"vendor":"B","number":"B1","issued":"2026-11-12","currency":"AUD","details":[{"position":"1","description":"service B","units":"1","charge":"50"}]}'
MAPPING = "id,supplier,raw,item\nm1,A,service A,shared\nm2,B,service B,shared\n"
RATES = "id,supplier,item,rate,start,end\nr1,A,shared,50,2026-01-01,2030-01-01\nr2,B,shared,45,2026-01-01,2030-01-01\n"
COMPONENTS = "id,supplier,invoice,line,amount\nc1,A,A1,1,1\nc2,A,A1,1,2\nc3,A,A1,1,3\nc4,B,B1,1,4\n"


def golden():
    return json.loads(GOLDEN.read_text(encoding="utf-8"))


def workbook(path, rows):
    """A minimal one-sheet ("Review") XLSX with inline-string cells."""
    xml_rows = []
    for number, values in enumerate(rows, 1):
        cells = "".join(
            f'<c r="{chr(65 + i)}{number}" t="inlineStr"><is><t>{value}</t></is></c>'
            for i, value in enumerate(values))
        xml_rows.append(f'<row r="{number}">{cells}</row>')
    parts = {
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>',
        "_rels/.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>',
        "xl/workbook.xml": '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><workbookPr date1904="0"/><sheets><sheet name="Review" sheetId="1" r:id="rId1"/></sheets></workbook>',
        "xl/_rels/workbook.xml.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>',
        "xl/worksheets/sheet1.xml": '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>' + "".join(xml_rows) + "</sheetData></worksheet>",
    }
    with ZipFile(path, "w", compression=ZIP_DEFLATED) as archive:
        for name, content in parts.items():
            archive.writestr(name, content)


def write_inputs(root, a_json=A_JSON, b_json=B_JSON, mapping=MAPPING, rates=RATES):
    root = Path(root)
    workbook(root / "enrichment.xlsx",
             [("id", "amount", "supplier"), ("1", "7.00", "A"), ("1", "8.00", "B"), ("2", "9.00", "A")])
    (root / "a.json").write_text(a_json, encoding="utf-8")
    (root / "b.json").write_text(b_json, encoding="utf-8")
    (root / "mapping.csv").write_text(mapping, encoding="utf-8")
    (root / "rates.csv").write_text(rates, encoding="utf-8")
    (root / "components.csv").write_text(COMPONENTS, encoding="utf-8")


def two_supplier_declaration():
    """The fixture recipe as portable data, built with iresiphi.build."""
    sources, relationships, outputs = [], [], []
    for id, array, supplier, invoice, date, line, item, quantity, billed in [
        ("a", "/lines", "/supplier", "/invoice", "/date", "/id", "/item", "/quantity", "/billed"),
        ("b", "/details", "/vendor", "/number", "/issued", "/position", "/description", "/units", "/charge"),
    ]:
        fields = {
            "supplier": b.field(supplier, b.string_type(), header=True),
            "invoice": b.field(invoice, b.string_type(), header=True),
            "date": b.field(date, b.date_type(), header=True),
            "currency": b.field("/currency", b.string_type(), header=True),
            "id": b.field(line, b.string_type()),
            "raw": b.field(item, b.string_type()),
            "quantity": b.field(quantity, b.decimal_type(12, 4)),
            "billed": b.field(billed, b.decimal_type(18, 2)),
        }
        sources.append(b.source(id, fields, ["supplier", "invoice", "id"], array=array))
        mapping, rate = f"{id}_mapping", f"{id}_rate"
        detail, enrichment = f"{id}_components", f"{id}_enrichment"
        relationships += [
            b.relationship(enrichment, id, "enrichment", ["supplier", "id"], ["supplier", "id"], "many_to_one"),
            b.relationship(mapping, id, "mapping", ["supplier", "raw"], ["supplier", "raw"], "many_to_one"),
            b.relationship(rate, id, "rates", ["supplier", f"{mapping}.item"], ["supplier", "item"],
                           "many_to_one", validity=b.validity("date", "start", "end")),
            b.relationship(detail, id, "components", ["supplier", "invoice", "id"],
                           ["supplier", "invoice", "line"], "one_to_many"),
        ]
        expected = b.quantize(b.multiply(b.ref(id, "quantity"), b.ref("rates", "rate", [rate])), 18, 2)
        outputs.append(b.output(f"{id}_review", id, [
            b.column("enriched", b.ref("enrichment", "amount", [enrichment])),
            b.column("id", b.ref(id, "id")),
            b.column("supplier", b.ref(id, "supplier")),
            b.column("billed", b.ref(id, "billed")),
            b.column("expected", expected),
            b.column("variance", b.subtract(b.ref(id, "billed"), expected)),
        ]))
    s = b.string_type
    sources.append(b.source("enrichment", {"id": b.field("id", s()), "supplier": b.field("supplier", s()),
                                           "amount": b.field("amount", b.decimal_type(18, 2))},
                            ["supplier", "id"]))
    sources.append(b.source("mapping", {k: b.field(k, s()) for k in ["id", "supplier", "raw", "item"]}, ["id"]))
    sources.append(b.source("rates", {
        "id": b.field("id", s()), "supplier": b.field("supplier", s()), "item": b.field("item", s()),
        "rate": b.field("rate", b.decimal_type(14, 6)), "start": b.field("start", b.date_type()),
        "end": b.field("end", b.date_type(), nullable=True)}, ["id"]))
    sources.append(b.source("components", {
        **{k: b.field(k, s()) for k in ["id", "supplier", "invoice", "line"]},
        "amount": b.field("amount", b.decimal_type(18, 2))}, ["id"]))
    return b.recipe("two suppliers", sources, outputs, relationships=relationships)


def bindings(root):
    root = Path(root)
    result = {
        id: b.binding(root / name, adapter)
        for id, name, adapter in [("a", "a.json", "json"), ("b", "b.json", "json"),
                                  ("mapping", "mapping.csv", "csv"), ("rates", "rates.csv", "csv"),
                                  ("components", "components.csv", "csv")]
    }
    result["enrichment"] = b.binding(root / "enrichment.xlsx", "xlsx", sheet="Review", header_row=1)
    return result


def policy(root, evidence="full", **limits):
    """The shared goldens count evidence for every line, so they state `full`."""
    return b.policy([Path(root)], [Path(root)], limits=limits or None, evidence=evidence)


def load_generator():
    """scripts/generate_fixture.py, the deterministic N-line workload generator."""
    spec = importlib.util.spec_from_file_location("generate_fixture", REPO / "scripts" / "generate_fixture.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def generated(root, lines):
    """Write an N-line workload under ``root``; return (recipe text, bindings, policy)."""
    import contextlib
    import io
    root = Path(root)
    with contextlib.redirect_stdout(io.StringIO()):
        load_generator().generate(root, lines)
    recipe = (root / "recipe.iresiphi").read_text(encoding="utf-8")
    bindings = json.loads((root / "bindings.json").read_text(encoding="utf-8"))
    policy = json.loads((root / "policy.json").read_text(encoding="utf-8"))
    return recipe, bindings, policy


# -- slice 2: the customer/month fixture (tests/support/customer_month.rs) ---------------

CUSTOMER_MONTH_GOLDEN = REPO / "tests" / "fixtures" / "customer_month.parity.json"
CUSTOMERS = "CustNo,SalesArea,Share\nA,North,1\nB,North,3\nC,South,1\n"
TRANSACTIONS = ("receipt_id,buyer_ref,booked_at,gross\nt1,A,2026-01-05,60\nt2,A,2026-01-20,40\n"
                "t3,B,2026-01-11,200\nt4,C,2026-01-17,50\nt5,B,2026-02-08,120\n")
TARGETS = ("Territory,Period,Goal\nNorth,2026-01,900\nSouth,2026-01,300\nNorth,2026-02,800\n"
           "South,2026-02,400\n")


def write_customer_month(root, customers=CUSTOMERS, transactions=TRANSACTIONS, targets=TARGETS):
    root = Path(root)
    (root / "customers.csv").write_text(customers, encoding="utf-8")
    (root / "transactions.csv").write_text(transactions, encoding="utf-8")
    (root / "targets.csv").write_text(targets, encoding="utf-8")


def customer_month_bindings(root):
    root = Path(root)
    return {id: b.binding(root / f"{id}.csv", "csv") for id in ["customers", "transactions", "targets"]}


def customer_month_declaration(recipient_filter=None, output_filter=None, nullable_share=False):
    """The full slice-2 fixture: sales aggregated onto customer × month, broadcast
    regional targets and targets allocated by Share."""
    s = b.string_type
    sources = [
        b.source("customers", {"CustNo": b.field("CustNo", s()), "SalesArea": b.field("SalesArea", s()),
                               "Share": b.field("Share", b.int64_type(), nullable=nullable_share)},
                 ["CustNo"]),
        b.source("transactions", {"receipt_id": b.field("receipt_id", s()),
                                  "buyer_ref": b.field("buyer_ref", s()),
                                  "booked_at": b.field("booked_at", b.date_type(), nullable=True),
                                  "gross": b.field("gross", b.decimal_type(18, 2))},
                 ["receipt_id"]),
        b.source("targets", {"Territory": b.field("Territory", s()), "Period": b.field("Period", s()),
                             "Goal": b.field("Goal", b.decimal_type(18, 2), nullable=True)},
                 ["Territory", "Period"]),
    ]
    population = b.population("customer_month", "customers",
                              b.months("month", values=["2026-01", "2026-02"]))
    relationships = [
        b.relationship("sales", "customer_month", "transactions", ["CustNo"], ["buyer_ref"],
                       "one_to_many", period=b.period_match("month", "booked_at")),
        b.relationship("region_target", "customer_month", "targets", ["SalesArea", "month"],
                       ["Territory", "Period"], "many_to_one"),
    ]
    columns = [
        b.column("CustNo", b.ref("customer_month", "CustNo")),
        b.column("month", b.ref("customer_month", "month")),
        b.column("currency", b.lit("AUD")),
        b.column("sales", b.aggregate("sales", "sum", "gross")),
        b.column("transactions", b.aggregate("sales", "count_records")),
        b.column("target_context", b.ref("targets", "Goal", ["region_target"])),
    ]
    allocated = b.allocation("allocated_target", ("targets", "Goal"), ["Territory", "Period"],
                             "region_target", b.ref("customer_month", "Share"), 18, 2,
                             "largest_remainder", recipient_filter=recipient_filter)
    output = b.output("customer_targets", "customer_month", columns, filter=output_filter,
                      complete=["sales"], allocations=[allocated])
    return b.recipe("customer month targets", sources, [output], relationships=relationships,
                    required_capabilities=["population", "allocation"], populations=[population])
