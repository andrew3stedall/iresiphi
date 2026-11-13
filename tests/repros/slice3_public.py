"""Public Python adversarial probes; run with an installed slice-3 wheel.

    python tests/repros/slice3_public.py

Assertions describe observed behavior, including the policy-builder defect.
"""
import csv
import json
from pathlib import Path
import sqlite3
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from python_sqlite import declaration, write_fixture
import iresiphi
from iresiphi import build as b


def compare(doc, bindings, root):
    with iresiphi.Recipe(doc) as recipe:
        snapshots = []
        for placement in ['local', 'prefer_source']:
            policy = b.policy([str(root)], placement=placement)
            with iresiphi.run(recipe, bindings, policy) as result:
                snapshots.append((result.batch('open_orders'), result.evidence(),
                                  result.evidence('candidate'), result.manifest['counts']))
        assert snapshots[0] == snapshots[1], json.dumps(doc)


def probes():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        bindings = write_fixture(root)
        with sqlite3.connect(root / 'sales.sqlite') as db:
            db.execute("UPDATE orders SET cust=NULL, booked=NULL WHERE order_no=2")
            db.execute("INSERT INTO orders VALUES (-9223372036854775808,'01','2026-01-01','1.00',0)")
            db.execute("INSERT INTO orders VALUES (9223372036854775807,'1','2026-01-01','1.00',1)")
        predicates = [b.is_null(b.ref('orders', 'cust')), b.is_null(b.ref('orders', 'open')),
                      b.not_(b.ref('orders', 'open')), b.lit(True), b.lit(False)]
        for field, values in [('order_no', [-9223372036854775808, -1, 0, 9223372036854775807]),
                              ('cust', ['a', 'A', '01', '1', '1.0', '\x00', 'é', '😀'])]:
            for value in values:
                for op in [b.equal, b.less, b.less_equal]:
                    predicates.append(op(b.ref('orders', field), b.lit(value)))
        for predicate in predicates:
            for wrapped in [predicate, b.not_(predicate), b.is_null(predicate),
                            b.and_(predicate, b.ref('orders', 'open')),
                            b.or_(predicate, b.is_null(b.ref('orders', 'booked')))]:
                compare(declaration(wrapped), bindings, root)
        print('205 predicate/null/int64/text parity probes passed')
        # Force a NOCASE text column and NUMERIC integer column, with quoted paths.
        with sqlite3.connect(root / 'sales.sqlite') as db:
            db.executescript('ALTER TABLE orders RENAME TO old; CREATE TABLE orders (order_no NUMERIC, cust TEXT COLLATE NOCASE, booked TEXT, net TEXT, open INTEGER); INSERT INTO orders SELECT * FROM old; DROP TABLE old; ALTER TABLE orders RENAME COLUMN cust TO "c""ust";')
        for predicate in [b.equal(b.ref('orders', 'cust'), b.lit('a')),
                          b.less(b.ref('orders', 'order_no'), b.lit(9223372036854775807))]:
            doc = declaration(predicate)
            doc['sources'][0]['projection']['fields']['cust']['path'] = 'c"ust'
            compare(doc, bindings, root)
        print('NOCASE, NUMERIC and quoted-column parity passed')
        with sqlite3.connect(root / 'sales.sqlite') as db:
            db.execute('ALTER TABLE orders RENAME COLUMN "c""ust" TO cust')
        with iresiphi.Recipe(declaration()) as recipe:
            for bad in ['unknown', 1, False, None, [], {}]:
                policy = b.policy([str(root)])
                policy['placement'] = bad
                try:
                    recipe.explain(bindings, policy)
                except iresiphi.IresiphiError as e:
                    assert e.code == 'INVALID_POLICY', (bad, e.code)
                else:
                    raise AssertionError(('invalid policy accepted', bad))
            for limits in [{'file_bytes': 1}, {'input_bytes': 1}, {'anchor_rows': 1}]:
                try:
                    iresiphi.run(recipe, bindings, b.policy([str(root)], limits=limits, placement='prefer_source'))
                except iresiphi.IresiphiError as e:
                    assert e.code == 'RESOURCE_LIMIT', e.code
                else:
                    raise AssertionError(('limit ignored', limits))
            try:
                iresiphi.run(recipe, bindings, b.policy([], placement='prefer_source'))
            except iresiphi.IresiphiError as e:
                assert e.code == 'POLICY_DENIED', e.code
            else:
                raise AssertionError('read roots ignored')
        print('raw policy JSON rejects unknown/wrong types; byte/row/read-root limits enforced')
        # Reuse guards must refuse before reading either binding.
        docs = []
        doc = declaration()
        duplicate = dict(doc['outputs'][0], id='second')
        doc['outputs'].append(duplicate)
        docs.append(doc)
        doc = declaration()
        doc['relationships'] = [b.relationship('reverse', 'customers', 'orders',
                                                   ['CustNo'], ['cust'], 'one_to_many')]
        doc['outputs'][0]['columns'] = doc['outputs'][0]['columns'][:3]
        docs.append(doc)
        doc = declaration()
        doc['populations'] = [b.population('members', 'orders', b.months('month', ['2026-01']))]
        doc['required_capabilities'] = ['population']
        docs.append(doc)
        doc = declaration()
        doc['features'] = [b.feature_declaration('active', b.ref('orders', 'open'))]
        doc['outputs'][0]['eligibility'] = b.feature('active')
        docs.append(doc)
        for doc in docs:
            with iresiphi.Recipe(doc) as recipe:
                explanation = recipe.explain(bindings, b.policy([str(root)], placement='prefer_source'))
                assert explanation['bound']['placement']['operations'][0]['placement'] == 'local'
                assert not recipe.explain(bindings, b.policy([str(root)], placement='require_source'))['bound']['feasible']
                compare(doc, bindings, root)
        doc = declaration()
        doc['outputs'][0]['filter'] = b.less(b.ref('orders', 'order_no'), b.lit(4))
        compare(doc, bindings, root)
        print('multiple-output, relationship-target, population, feature guards and output-filter parity passed')
        with iresiphi.Recipe(declaration()) as recipe:
            explanation = recipe.explain(bindings, b.policy([str(root)], placement='require_source'))
            assert explanation['bound']['placement']['policy'] == 'require_source'
            assert explanation['bound']['placement']['operations'][0]['policy'] == 'prefer_source'
            print('REPRODUCED: require_source explain operation reports prefer_source policy')
        # Slice-2 aggregates, broadcast targets and allocations still see all rows
        # when an extra output asks to filter one of their backing sources.
        import python_support as support
        support.write_customer_month(root)
        month_bindings = support.customer_month_bindings(root)
        month_doc = support.customer_month_declaration()
        with sqlite3.connect(root / 'month.sqlite') as db:
            for source in month_doc['sources']:
                fields = source['projection']['fields']
                names = list(fields)
                quoted = lambda n: '"' + n.replace('"', '""') + '"'
                db.execute('CREATE TABLE ' + quoted(source['id']) + ' (' + ','.join(quoted(n) for n in names) + ')')
                with (root / (source['id'] + '.csv')).open() as f:
                    for row in csv.DictReader(f):
                        values = [int(row[n]) if fields[n]['data_type']['kind'] == 'int64' else row[n] for n in names]
                        db.execute('INSERT INTO ' + quoted(source['id']) + ' VALUES (' + ','.join('?' for _ in names) + ')', values)
                month_bindings[source['id']] = b.binding(str(root / 'month.sqlite'), 'sqlite', table=source['id'])
        for source, field, value in [('customers', 'CustNo', 'A'), ('transactions', 'receipt_id', 't1'), ('targets', 'Territory', 'North')]:
            doc = support.customer_month_declaration()
            doc['outputs'].append(b.output('probe', source, [b.column(field, b.ref(source, field))],
                                          eligibility=b.equal(b.ref(source, field), b.lit(value))))
            with iresiphi.Recipe(doc) as recipe:
                snapshots = []
                for placement in ['local', 'prefer_source']:
                    with iresiphi.run(recipe, month_bindings, b.policy([str(root)], placement=placement)) as result:
                        snapshots.append(([result.batch(o) for o in sorted(result.outputs)],
                                          [result.evidence(k) for k in ['primary', 'candidate', 'detail', 'allocation']],
                                          result.manifest['counts']))
                assert snapshots[0] == snapshots[1], source
        print('aggregate, broadcast and allocation source-reuse parity passed')
        with sqlite3.connect(root / 'sales.sqlite') as db:
            db.execute('ALTER TABLE orders RENAME COLUMN cust TO "/cust"')
        doc = declaration()
        doc['sources'][0]['projection']['fields']['cust']['path'] = '/cust'
        with iresiphi.Recipe(doc) as recipe:
            try:
                iresiphi.run(recipe, bindings, b.policy([str(root)]))
            except iresiphi.IresiphiError as e:
                assert e.code == 'INVALID_PROJECTION', e.code
                assert 'column cust is not present' in str(e), str(e)
            else:
                raise AssertionError('slash column now supported: update defect reproduction')
        print('REPRODUCED: exact SQLite column path /cust is rewritten to cust and rejected')
        # The convenience builder silently discards invalid falsy placement values.
        for bad in [False, 0, '', [], {}]:
            assert 'placement' not in b.policy([str(root)], placement=bad)
        print('REPRODUCED: build.policy silently omits invalid falsy placement values')
        negative = root / 'negative'
        negative.mkdir()
        negative_bindings = write_fixture(negative)
        for update in ["UPDATE orders SET booked='2026-1-05' WHERE order_no=1",
                       "UPDATE orders SET open=2 WHERE order_no=2",
                       "UPDATE orders SET net=1.5 WHERE order_no=2",
                       "UPDATE orders SET cust=NULL WHERE order_no=1"]:
            # Each case starts with the same fixture.
            with sqlite3.connect(negative / 'sales.sqlite') as db:
                db.execute('DELETE FROM orders')
                from python_sqlite import ORDERS
                db.executemany('INSERT INTO orders VALUES (?,?,?,?,?)', ORDERS)
                db.execute(update)
            doc = declaration()
            if 'cust=NULL' in update:
                doc['sources'][0]['projection']['fields']['cust']['nullable'] = False
            codes = []
            with iresiphi.Recipe(doc) as recipe:
                for placement in ['local', 'prefer_source']:
                    try:
                        with iresiphi.run(recipe, negative_bindings, b.policy([str(root)], placement=placement)):
                            codes.append('success')
                    except iresiphi.IresiphiError as e:
                        codes.append(e.code)
            assert codes[0] == codes[1], (update, codes)
        doc = declaration()
        doc['sources'][0]['projection']['fields']['order_no']['path'] = 'ORDER_NO'
        with iresiphi.Recipe(doc) as recipe:
            report = recipe.validate(negative_bindings, b.policy([str(root)]))
            assert not report['valid']
            assert 'INVALID_PROJECTION' in json.dumps(report)
        print('non-ISO date, invalid Boolean/required NULL, decimal text and exact-case schema probes passed')


if __name__ == '__main__':
    probes()
