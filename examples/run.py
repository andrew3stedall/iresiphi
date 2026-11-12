"""Save, reopen, rebind, validate, explain and run a recipe from Python.

    python examples/run.py RECIPE BINDINGS POLICY WORK_DIR [SOURCE=PATH ...]

The same steps as examples/run.rs, through the installed ``iresiphi`` wheel:

1. Reopen the saved recipe RECIPE, save it again as WORK_DIR/saved.iresiphi,
   reopen that copy and check that the semantic hash is unchanged.
2. Validate the reopened recipe without bindings (static checks), then with
   BINDINGS and POLICY (access and source schemas; no data checks).
3. Explain it with the bindings: logical nodes, lineage, reads and budgets.
4. Run it into a new bundle, WORK_DIR/bundle-1, and read a few exact values.
5. If SOURCE=PATH pairs are given, rebind those sources to new files, keeping
   their adapter and settings, and run the unchanged recipe again into
   WORK_DIR/bundle-2. The saved recipe is not edited; only bindings change.

WORK_DIR must not exist yet and must lie under an allowed output root of POLICY.
"""
import copy
import json
import sys
from pathlib import Path

import iresiphi


def run(context, recipe, bindings, policy, bundle):
    # A failed or cancelled run raises IresiphiError (or RunCancelled) carrying
    # the run report; nothing is published.
    with context.run(recipe, bindings, policy, destination=str(bundle)) as result:
        manifest = result.manifest
        print(f"run {result.run_id}: {result.status} into {bundle}; "
              f"{manifest['resolved_rows']} resolved and {manifest['unresolved_rows']} unresolved lines; "
              f"reconciliation {manifest['reconciliation']['status']}")
        print(f"  totals: {json.dumps(manifest['totals'])}")
        for d in result.diagnostics:
            print(f"  {d['severity']} {d['code']} {d.get('subject') or '-'}: {d.get('affected_count')} records")
        # Values are copied out as exact Python objects (Decimal, date, str, None)
        # and stay valid after the result is closed.
        first_output = result.outputs[0]
        first_batch = next(iter(result.batches(first_output, rows=2)), [])
        for row in first_batch:
            print(f"  {first_output}: billed={row.get('billed')!r} expected={row.get('expected')!r} "
                  f"status={row.get('_status')}")
        return manifest


def main(argv):
    if len(argv) < 5:
        sys.exit("usage: run.py RECIPE BINDINGS POLICY WORK_DIR [SOURCE=PATH ...]")
    bindings = json.loads(Path(argv[2]).read_text(encoding="utf-8"))
    policy = json.loads(Path(argv[3]).read_text(encoding="utf-8"))
    work = Path(argv[4])
    work.mkdir()

    # 1. Save and reopen. A saved recipe holds no paths, data or secrets.
    opened = iresiphi.load(argv[1])
    saved = work / "saved.iresiphi"
    iresiphi.save(opened, saved)
    recipe = iresiphi.load(saved)
    assert recipe.hash == opened.hash, "reopening preserves meaning"
    print(f"saved and reopened {saved} ({recipe.hash})")

    # 2. Validate: static, then bound (access and schemas only).
    for label, report in [("static", recipe.validate()),
                          ("bound", recipe.validate(bindings, policy, "none"))]:
        failed = [c for c in report["checks"] if c["status"] == "failed"]
        print(f"validate ({label}): valid={report['valid']} checks={len(report['checks'])} failed={len(failed)}")
        if not report["valid"]:
            sys.exit(f"{label} validation failed: {failed}")

    # 3. Explain with bindings. Nothing is read; estimates stay unknown.
    explanation = recipe.explain(bindings, policy)
    print(f"explain: {len(explanation['nodes'])} nodes, feasible={explanation['bound']['feasible']}")

    # 4. Run into a new bundle.
    context = iresiphi.ExecutionContext()
    first = run(context, recipe, bindings, policy, work / "bundle-1")

    # 5. Rebind the unchanged recipe and run again.
    if len(argv) > 5:
        rebound = copy.deepcopy(bindings)
        for pair in argv[5:]:
            source, _, path = pair.partition("=")
            rebound[source]["path"] = path
        second = run(context, recipe, rebound, policy, work / "bundle-2")
        print(f"rebound run: recipe hash unchanged={second['recipe_hash'] == first['recipe_hash']}")
    recipe.close()


if __name__ == "__main__":
    main(sys.argv)
