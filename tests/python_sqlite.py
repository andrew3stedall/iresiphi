"""Slice 3 placement acceptance through the installed public Python API."""
import datetime
import sqlite3
import tempfile
import unittest
from decimal import Decimal
from pathlib import Path

import iresiphi
from iresiphi import build as b

ORDERS = [(1, 'A', '2026-01-05', '60.00', 1),
          (2, 'A', '2026-01-20', '40.00', 0),
          (3, 'B', '2026-01-11', '200.00', 1),
          (4, 'C', '2026-02-08', '120.50', 1),
          (5, 'Z', '2026-02-09', '10.00', 1),
          (6, 'B', '2025-12-31', '75.25', 1)]


def declaration(eligibility=None):
    return b.recipe('open orders', [
        b.source('orders', {
            'order_no': b.field('order_no', b.int64_type()),
            'cust': b.field('cust', b.string_type(), nullable=True),
            'booked': b.field('booked', b.date_type(), nullable=True),
            'net': b.field('net', b.decimal_type(18, 2)),
            'open': b.field('open', b.boolean_type()),
        }, ['order_no']),
        b.source('customers', {
            'CustNo': b.field('CustNo', b.string_type()),
            'SalesArea': b.field('SalesArea', b.string_type()),
        }, ['CustNo']),
    ], [b.output('open_orders', 'orders', [
        b.column('order_no', b.ref('orders', 'order_no')),
        b.column('currency', b.lit('AUD')),
        b.column('net', b.ref('orders', 'net')),
        b.column('area', b.ref('customers', 'SalesArea', ['order_customer'])),
    ], eligibility=eligibility if eligibility is not None else b.and_(
        b.ref('orders', 'open'),
        b.less_equal(b.lit(datetime.date(2026, 1, 1)), b.ref('orders', 'booked'))))],
        relationships=[b.relationship('order_customer', 'orders', 'customers',
                                      ['cust'], ['CustNo'], 'many_to_one')])


def write_fixture(root):
    with sqlite3.connect(root / 'sales.sqlite') as db:
        db.execute('CREATE TABLE orders (order_no INTEGER, cust TEXT, booked TEXT, net TEXT, open INTEGER)')
        db.executemany('INSERT INTO orders VALUES (?, ?, ?, ?, ?)', ORDERS)
    (root / 'customers.csv').write_text('CustNo,SalesArea\nA,North\nB,North\nC,South\n', encoding='utf-8')
    return {'orders': b.binding(str(root / 'sales.sqlite'), 'sqlite', table='orders'),
            'customers': b.binding(str(root / 'customers.csv'), 'csv')}


class SQLitePlacement(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.bindings = write_fixture(self.root)

    def policy(self, placement):
        return b.policy([str(self.root)], placement=placement)

    def test_acceptance_placement_preserves_rows_evidence_counts_and_recipe(self):
        with iresiphi.Recipe(declaration()) as recipe:
            original = recipe.hash, recipe.canonical_json()
            snapshots = []
            for placement in ['local', 'prefer_source', 'require_source']:
                policy = self.policy(placement)
                self.assertTrue(recipe.validate(self.bindings, policy, 'full')['valid'])
                explanation = recipe.explain(self.bindings, policy)['bound']
                self.assertTrue(explanation['feasible'])
                self.assertEqual(explanation['placement']['operations'][0]['placement'],
                                 'local' if placement == 'local' else 'source')
                with iresiphi.run(recipe, self.bindings, policy) as result:
                    rows = sorted(result.batch('open_orders'), key=lambda r: r['order_no'])
                    self.assertEqual([r['order_no'] for r in rows], [1, 3, 4, 5])
                    self.assertEqual([str(r['net']) for r in rows], ['60.00', '200.00', '120.50', '10.00'])
                    self.assertEqual([r['area'] for r in rows], ['North', 'North', 'South', None])
                    self.assertEqual(rows[-1]['_status'], 'unmatched')
                    self.assertEqual(result.status, 'succeeded_with_unresolved')
                    source = result.manifest['sources']['orders']
                    self.assertEqual(source['rows'], 6)
                    self.assertEqual(source.get('scanned_rows'), None if placement == 'local' else 4)
                    snapshots.append((rows, result.evidence(), result.evidence('candidate'),
                                      result.manifest['counts']['open_orders']))
                self.assertEqual((recipe.hash, recipe.canonical_json()), original)
            self.assertEqual(snapshots[0], snapshots[1])
            self.assertEqual(snapshots[0], snapshots[2])

    def test_colocated_reduction_public_fixture(self):
        with sqlite3.connect(self.root / 'sales.sqlite') as db:
            db.execute("INSERT INTO orders VALUES(7,'D','2025-11-02','30.00',1)")
            db.execute('CREATE TABLE customers(CustNo TEXT, SalesArea TEXT, Since TEXT)')
            db.executemany('INSERT INTO customers VALUES(?,?,?)', [
                ('A', 'North', '2020-01-01'), ('B', 'North', None),
                ('C', 'South', '2021-06-30'), ('D', 'West', '2022-02-02'),
                ('E', 'East', '2023-03-03')])
        self.bindings['customers'] = b.binding(str(self.root / 'sales.sqlite'), 'sqlite', table='customers')
        doc = declaration()
        doc['sources'][1]['projection']['fields']['Since'] = b.field('Since', b.date_type(), nullable=True)
        with iresiphi.Recipe(doc) as recipe:
            original = recipe.hash, recipe.canonical_json()
            snapshots = []
            for placement in ['local', 'prefer_source', 'require_source']:
                policy = self.policy(placement)
                policy['limits'] = {'input_bytes': (self.root / 'sales.sqlite').stat().st_size}
                self.assertTrue(recipe.validate(self.bindings, policy, 'full')['valid'])
                self.assertTrue(recipe.explain(self.bindings, policy)['bound']['feasible'])
                with iresiphi.run(recipe, self.bindings, policy) as result:
                    sources = result.manifest['sources']
                    self.assertEqual(sources['orders']['shared_read'], ['customers'])
                    self.assertEqual(sources['customers']['shared_read'], ['orders'])
                    self.assertEqual(sources['orders']['fingerprint'], sources['customers']['fingerprint'])
                    self.assertEqual(sources['orders']['rows'], 7)
                    self.assertEqual(sources['customers']['rows'], 5)
                    self.assertEqual(sources['customers'].get('scanned_rows'), None if placement == 'local' else 3)
                    snapshots.append((result.batch('open_orders'), result.evidence(), result.evidence('candidate'), result.manifest['counts']))
            self.assertEqual(snapshots[0], snapshots[1])
            self.assertEqual(snapshots[0], snapshots[2])
            self.assertEqual((recipe.hash, recipe.canonical_json()), original)

    def test_key_transfer_reduces_a_sqlite_target_from_csv_orders(self):
        """Slice 5: orders in CSV, customers in SQLite, keys sent under policy."""
        (self.root / 'orders.csv').write_text(
            'order_no,cust,booked,net,open\n' + ''.join(
                f'{n},{c},{d},{net},{"true" if o else "false"}\n'
                for n, c, d, net, o in ORDERS + [(7, 'D', '2025-11-02', '30.00', 1)]),
            encoding='utf-8')
        with sqlite3.connect(self.root / 'crm.sqlite') as db:
            db.execute('CREATE TABLE customers (CustNo TEXT, SalesArea TEXT, Since TEXT)')
            db.executemany('INSERT INTO customers VALUES (?, ?, ?)', [
                ('A', 'North', '2020-01-01'), ('B', 'North', None),
                ('C', 'South', '2021-06-30'), ('D', 'West', '2022-02-02'),
                ('E', 'East', '2023-03-03')])
        bindings = {'orders': b.binding(str(self.root / 'orders.csv'), 'csv'),
                    'customers': b.binding(str(self.root / 'crm.sqlite'), 'sqlite', table='customers')}
        doc = declaration()
        doc['sources'][1]['projection']['fields']['Since'] = b.field('Since', b.date_type(), nullable=True)
        with iresiphi.Recipe(doc) as recipe:
            original = recipe.hash, recipe.canonical_json()
            snapshots = []
            for placement, transfers, scanned in [
                    ('local', None, None), ('require_source', None, None), ('prefer_source', 'none', None),
                    ('local', 'keys', None), ('prefer_source', 'keys', 4), ('require_source', 'keys', 4)]:
                policy = b.policy([str(self.root)], placement=placement, transfers=transfers)
                self.assertTrue(recipe.validate(bindings, policy, 'full')['valid'])
                bound = recipe.explain(bindings, policy)['bound']
                self.assertTrue(bound['feasible'])
                self.assertEqual([t['transfer'] for t in bound['transfers']],
                                 ['relationship:order_customer/keys'] if scanned else [])
                with iresiphi.run(recipe, bindings, policy) as result:
                    customers = result.manifest['sources']['customers']
                    self.assertEqual(customers['rows'], 5)
                    self.assertEqual(customers.get('scanned_rows'), scanned)
                    if scanned:
                        self.assertEqual(customers['placement']['transfer']['keys'], 5)
                        self.assertEqual(customers['placement']['reduction'], 'transferred_keys')
                    self.assertEqual([r['order_no'] for r in result.batch('open_orders')], [1, 3, 4, 5])
                    snapshots.append((result.batch('open_orders'), result.evidence(),
                                      result.evidence('candidate'), result.manifest['counts']))
            for snapshot in snapshots[1:]:
                self.assertEqual(snapshot, snapshots[0])
            limited = b.policy([str(self.root)], placement='require_source', transfers='keys',
                               limits={'transfer_keys': 4})
            with self.assertRaises(iresiphi.IresiphiError) as raised:
                iresiphi.run(recipe, bindings, limited)
            self.assertEqual(raised.exception.code, 'PLACEMENT_INFEASIBLE')
            self.assertEqual((recipe.hash, recipe.canonical_json()), original)
        self.assertNotIn('transfers', b.policy([str(self.root)]))

    def test_falsy_placement_is_preserved_and_rejected(self):
        for placement in ['', False, 0]:
            policy = self.policy(placement)
            self.assertEqual(policy['placement'], placement)
            with iresiphi.Recipe(declaration()) as recipe:
                with self.assertRaises((iresiphi.IresiphiError, ValueError, TypeError)):
                    recipe.validate(self.bindings, policy)
        self.assertNotIn('placement', b.policy([str(self.root)]))

    def test_decimal_falls_back_or_refuses_before_opening(self):
        predicate = b.less(b.lit(Decimal('50.00'), 18, 2), b.ref('orders', 'net'))
        with iresiphi.Recipe(declaration(predicate)) as recipe:
            with iresiphi.run(recipe, self.bindings, self.policy('prefer_source')) as result:
                self.assertEqual(result.manifest['sources']['orders'].get('scanned_rows'), None)
                self.assertEqual([r['order_no'] for r in result.batch('open_orders')], [1, 3, 4, 6])
            (self.root / 'sales.sqlite').unlink()
            explanation = recipe.explain(self.bindings, self.policy('prefer_source'))['bound']
            self.assertTrue(explanation['feasible'])
            self.assertIn('decimal', explanation['placement']['operations'][0]['reason'])
            self.assertFalse(recipe.validate(self.bindings, self.policy('require_source'))['valid'])
            self.assertFalse(recipe.explain(self.bindings, self.policy('require_source'))['bound']['feasible'])
            with self.assertRaises(iresiphi.IresiphiError) as raised:
                iresiphi.run(recipe, self.bindings, self.policy('require_source'))
            self.assertEqual(raised.exception.code, 'PLACEMENT_INFEASIBLE')


if __name__ == '__main__':
    unittest.main()
