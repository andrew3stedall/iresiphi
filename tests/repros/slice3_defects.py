"""Minimal public-API reproductions of three slice-3 review findings.

Run with an installed wheel: python tests/repros/slice3_defects.py [policy|explain|column]
Assertions confirm the defects remain reproducible; this is not a CI regression suite.
"""
from pathlib import Path
import sqlite3
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from python_sqlite import declaration, write_fixture
import iresiphi
from iresiphi import build as b


def reproduce(case):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        bindings = write_fixture(root)
        if case == 'policy':
            assert 'placement' not in b.policy([directory], placement=False)
            # The same invalid value supplied directly is rejected by Rust.
            with iresiphi.Recipe(declaration()) as recipe:
                try:
                    recipe.explain(bindings, {**b.policy([directory]), 'placement': False})
                except iresiphi.IresiphiError as error:
                    assert error.code == 'INVALID_POLICY'
                else:
                    raise AssertionError('expected INVALID_POLICY')
        elif case == 'explain':
            with iresiphi.Recipe(declaration()) as recipe:
                placement = recipe.explain(bindings, b.policy([directory], placement='require_source'))['bound']['placement']
                assert placement['policy'] == 'require_source'
                assert placement['operations'][0]['policy'] == 'prefer_source'
        elif case == 'column':
            with sqlite3.connect(root / 'sales.sqlite') as db:
                db.execute('ALTER TABLE orders RENAME COLUMN cust TO "/cust"')
            doc = declaration()
            doc['sources'][0]['projection']['fields']['cust']['path'] = '/cust'
            with iresiphi.Recipe(doc) as recipe:
                try:
                    iresiphi.run(recipe, bindings, b.policy([directory]))
                except iresiphi.IresiphiError as error:
                    assert error.code == 'INVALID_PROJECTION'
                    assert 'column cust is not present' in str(error)
                else:
                    raise AssertionError('expected exact /cust binding to fail')
        else:
            raise ValueError(case)
        print(f'REPRODUCED: {case}')


if __name__ == '__main__':
    for case in sys.argv[1:] or ['policy', 'explain', 'column']:
        reproduce(case)
