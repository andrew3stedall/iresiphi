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
