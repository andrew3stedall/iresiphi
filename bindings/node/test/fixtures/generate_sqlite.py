"""Regenerate sales.sqlite for Node 22/24 tests with Python's stdlib SQLite.

Run from any directory: python3 bindings/node/test/fixtures/generate_sqlite.py
No Node SQLite API or external SQLite package is needed at test runtime.
"""
import sqlite3
from pathlib import Path

path = Path(__file__).with_name('sales.sqlite')
path.unlink(missing_ok=True)
with sqlite3.connect(path) as db:
    db.execute('CREATE TABLE orders (order_no INTEGER, cust TEXT, booked TEXT, net TEXT, open INTEGER)')
    db.executemany('INSERT INTO orders VALUES (?, ?, ?, ?, ?)', [
        (1, 'A', '2026-01-05', '60.00', 1),
        (2, 'A', '2026-01-20', '40.00', 0),
        (3, 'B', '2026-01-11', '200.00', 1),
        (4, 'C', '2026-02-08', '120.50', 1),
        (5, 'Z', '2026-02-09', '10.00', 1),
        (6, 'B', '2025-12-31', '75.25', 1),
    ])

# Slice 4 uses the same orders plus an old D order and a co-located target.
import shutil

colocated = path.with_name('sales-colocated.sqlite')
shutil.copyfile(path, colocated)
with sqlite3.connect(colocated) as db:
    db.execute("INSERT INTO orders VALUES (7, 'D', '2025-11-02', '30.00', 1)")
    db.execute('CREATE TABLE customers (CustNo TEXT, SalesArea TEXT, Since TEXT)')
    db.executemany('INSERT INTO customers VALUES (?, ?, ?)', [
        ('A', 'North', '2020-01-01'),
        ('B', 'North', None),
        ('C', 'South', '2021-06-30'),
        ('D', 'West', '2022-02-02'),
        ('E', 'East', '2023-03-03'),
    ])
