#!/usr/bin/env python3
"""Create deterministic local invoice-review fixtures using only Python's stdlib.

Example: python scripts/generate_fixture.py /tmp/iresiphi-tiny
         python scripts/generate_fixture.py /tmp/iresiphi-100k --lines 100000
The seed identifies deterministic arithmetic generation (no PRNG is used): the same
seed and --lines always produce byte-identical data files and recipe, so the
`fixture_sha256` printed and written to fixture-manifest.json identifies the fixture.

Shape: supplier A and B lines alternate; each supplier has one JSON document whose
lines carry their own invoice identifier (a new invoice every 100 generated
lines, so 50 lines per invoice). Line i has i % 4
observed components (0 to 3), so component counts vary from line to line. Line 3 has
an unmapped item and stays unresolved. Every line has an XLSX enrichment row.
"""
import argparse
import csv
import hashlib
import json
from pathlib import Path
from xml.sax.saxutils import escape
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

SEED = "iresiphi-preview-v1"

def write_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")

def write_csv(path, header, records):
    with path.open("w", encoding="utf-8", newline="") as stream:
        writer = csv.writer(stream, lineterminator="\n")
        writer.writerow(header)
        writer.writerows(records)

def workbook(path, records):
    rows = []
    for number, values in enumerate([("id", "amount", "supplier"), *records], 1):
        cells = "".join(f'<c r="{chr(65+i)}{number}" t="inlineStr"><is><t>{escape(str(value))}</t></is></c>' for i, value in enumerate(values))
        rows.append(f'<row r="{number}">{cells}</row>')
    files = {
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>',
        "_rels/.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>',
        "xl/workbook.xml": '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Review" sheetId="1" r:id="rId1"/></sheets></workbook>',
        "xl/_rels/workbook.xml.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>',
        "xl/worksheets/sheet1.xml": '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>' + "".join(rows) + '</sheetData></worksheet>',
    }
    with ZipFile(path, "w", compression=ZIP_DEFLATED) as archive:
        for name, content in files.items():
            info = ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = ZIP_DEFLATED
            info.external_attr = 0o600 << 16
            archive.writestr(info, content.encode("utf-8"))

def typed(path, kind="string", header=False, nullable=False, **kwargs):
    return {"path": path, "data_type": {"kind": kind, **kwargs}, "header": header, "nullable": nullable}

def field(source, name, path=()):
    return {"op": "field", "source": source, "field": name, "path": list(path)}

def source(id, fields, identity, array=None):
    return {"id": id, "projection": {"array": array, "fields": fields}, "identity": identity, "grain": identity}

def make_recipe():
    sources, relationships, outputs = [], [], []
    for id, array, supplier, date, line, raw, quantity, billed in [
        ("a", "/lines", "/supplier", "/date", "/id", "/item", "/quantity", "/billed"),
        ("b", "/details", "/vendor", "/issued", "/position", "/description", "/units", "/charge"),
    ]:
        sources.append(source(id, {
            "supplier": typed(supplier, header=True), "invoice": typed("/invoice"),
            "date": typed(date, "date", header=True), "currency": typed("/currency", header=True),
            "id": typed(line), "raw": typed(raw),
            "quantity": typed(quantity, "decimal", precision=12, scale=4),
            "billed": typed(billed, "decimal", precision=18, scale=2),
        }, ["supplier", "invoice", "id"], array))
        for name, target, left, right, cardinality in [
            (f"{id}_mapping", "mapping", ["supplier", "raw"], ["supplier", "raw"], "many_to_one"),
            (f"{id}_rate", "rates", ["supplier", f"{id}_mapping.item"], ["supplier", "item"], "many_to_one"),
            (f"{id}_enrichment", "enrichment", ["supplier", "id"], ["supplier", "id"], "many_to_one"),
            (f"{id}_components", "components", ["supplier", "invoice", "id"], ["supplier", "invoice", "line"], "one_to_many"),
        ]:
            rel = {"id": name, "from": id, "to": target, "from_keys": left, "to_keys": right, "cardinality": cardinality}
            if target == "rates":
                rel["validity"] = {"date": "date", "valid_from": "start", "valid_to": "end"}
            relationships.append(rel)
        expected = {"op": "quantize", "expr": {"op": "multiply", "left": field(id, "quantity"), "right": field("rates", "rate", [f"{id}_rate"])}, "precision": 18, "scale": 2, "rounding": "half_even"}
        columns = [{"name": name, "expr": field(id, name)} for name in ["supplier", "invoice", "id", "currency", "billed"]]
        columns += [
            {"name": "enriched", "expr": field("enrichment", "amount", [f"{id}_enrichment"])},
            {"name": "expected", "expr": expected},
            {"name": "variance", "expr": {"op": "subtract", "left": field(id, "billed"), "right": expected}},
        ]
        outputs.append({"id": f"{id}_review", "anchor": id, "columns": columns})
    sources.append(source("mapping", {key: typed(key) for key in ["id", "supplier", "raw", "item"]}, ["id"]))
    sources.append(source("rates", {**{key: typed(key) for key in ["id", "supplier", "item"]}, "rate": typed("rate", "decimal", precision=14, scale=6), "start": typed("start", "date"), "end": typed("end", "date", nullable=True)}, ["id"]))
    sources.append(source("components", {**{key: typed(key) for key in ["id", "supplier", "invoice", "line"]}, "amount": typed("amount", "decimal", precision=18, scale=2)}, ["id"]))
    sources.append(source("enrichment", {"id": typed("id"), "supplier": typed("supplier"), "amount": typed("amount", "decimal", precision=18, scale=2)}, ["supplier", "id"]))
    return {"format_version": "0.1", "name": "Auditable two-supplier invoice review", "sources": sources, "relationships": list(reversed(relationships)), "features": [], "outputs": outputs, "annotations": {"fixture_seed": SEED}}

def generate(root, count, byte_limit=None, evidence=None):
    root.mkdir(parents=True, exist_ok=True)
    a, b, components, enrichment = [], [], [], []
    full, comparable, expected_total = 0, 0, 0
    for index in range(count):
        supplier = "A" if index % 2 == 0 else "B"
        line = str(index + 1)
        invoice = f"{supplier}{index // 100 + 1}"
        unknown = index == 2
        billed = 20 if unknown else 110 if supplier == "A" else 50
        quantity = 1 if unknown or supplier == "B" else 2
        raw = "unknown" if unknown else f"service {supplier}"
        full += billed
        if not unknown:
            comparable += billed
            expected_total += 100 if supplier == "A" else 45
        if supplier == "A":
            a.append({"invoice": invoice, "id": line, "item": raw, "quantity": str(quantity), "billed": str(billed)})
        else:
            b.append({"invoice": invoice, "position": line, "description": raw, "units": str(quantity), "charge": str(billed)})
        enrichment.append((line, "7.00" if supplier == "A" else "8.00", supplier))
        for component in range(3 if count == 3 and index == 0 else index % 4):
            components.append((f"c{index}-{component}", supplier, invoice, line, str(component + 1)))
    write_json(root / "a.json", {"supplier": "A", "date": "2026-11-12", "currency": "AUD", "lines": a})
    write_json(root / "b.json", {"vendor": "B", "issued": "2026-11-12", "currency": "AUD", "details": b})
    write_csv(root / "mapping.csv", ["id", "supplier", "raw", "item"], [("m1", "A", "service A", "shared"), ("m2", "B", "service B", "shared")])
    write_csv(root / "rates.csv", ["id", "supplier", "item", "rate", "start", "end"], [("r1", "A", "shared", "50", "2026-01-01", "2030-01-01"), ("r2", "B", "shared", "45", "2026-01-01", "2030-01-01")])
    write_csv(root / "components.csv", ["id", "supplier", "invoice", "line", "amount"], components)
    workbook(root / "enrichment.xlsx", enrichment)
    write_json(root / "recipe.iresiphi", make_recipe())
    bindings = {id: {"path": str(root / filename), "adapter": adapter, "settings": {}} for id, filename, adapter in [("a", "a.json", "json"), ("b", "b.json", "json"), ("mapping", "mapping.csv", "csv"), ("rates", "rates.csv", "csv"), ("components", "components.csv", "csv"), ("enrichment", "enrichment.xlsx", "xlsx")]}
    bindings["enrichment"]["settings"] = {"sheet": "Review", "header_row": 1}
    write_json(root / "bindings.json", bindings)
    write_json(root / "policy.json", {"allowed_read_roots": [str(root)], "allowed_output_roots": [str(root)], "limits": {} if byte_limit is None else {"output_bytes": byte_limit, "retained_bytes": byte_limit}, "diagnostic_values": False, **({} if evidence is None else {"evidence": evidence})})
    inputs = {name: {"sha256": hashlib.sha256((root / name).read_bytes()).hexdigest(), "bytes": (root / name).stat().st_size} for name in ["a.json", "b.json", "mapping.csv", "rates.csv", "components.csv", "enrichment.xlsx", "recipe.iresiphi"]}
    # One digest for the whole fixture: SHA-256 of the compact, key-sorted JSON map
    # of file name to file SHA-256. bindings.json and policy.json hold absolute
    # paths, so they are excluded; regenerate them for each directory.
    fixture_sha256 = hashlib.sha256(json.dumps({name: entry["sha256"] for name, entry in inputs.items()}, sort_keys=True, separators=(",", ":")).encode("utf-8")).hexdigest()
    invoices = len({line["invoice"] for line in a} | {line["invoice"] for line in b})
    evidence = {"seed": SEED, "fixture_sha256": fixture_sha256, "line_count": count, "invoice_count": invoices, "component_count": len(components), "expected": {"full_billed": str(full), "comparable_billed": str(comparable), "expected": str(expected_total), "variance": str(comparable - expected_total), "unresolved_billed": str(full - comparable), "unresolved_count": 1 if count >= 3 else 0}, "inputs": inputs}
    write_json(root / "fixture-manifest.json", evidence)
    print(json.dumps(evidence, indent=2))

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--lines", type=int, default=3)
    parser.add_argument("--byte-limit", type=int, default=None, help="set the policy's output_bytes and retained_bytes limits (the 100,000-line workload uses 1073741824)")
    parser.add_argument("--evidence", choices=["problems", "full"], default=None, help="set the policy's evidence level (the default records only lines that did not resolve; full at 100,000 lines needs --byte-limit 1073741824)")
    args = parser.parse_args()
    if args.lines < 3:
        parser.error("--lines must be at least 3")
    generate(args.directory.resolve(), args.lines, args.byte_limit, args.evidence)
