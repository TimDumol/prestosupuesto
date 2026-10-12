#!/usr/bin/env python3
"""Verify restored backup bytes and exact finance contents without writing to SQLite."""
import hashlib
import json
import sqlite3
import sys
from pathlib import Path

root = Path(sys.argv[1])
for line in (root / 'SHA256SUMS').read_text().splitlines():
    digest, name = line.split(None, 1)
    if hashlib.sha256((root / name).read_bytes()).hexdigest() != digest:
        raise RuntimeError(f'Checksum mismatch: {name}')
snapshot = json.loads((root / 'finance.json').read_text())
with sqlite3.connect(f'file:{root}/presto.sqlite?mode=ro', uri=True) as database:
    if database.execute('PRAGMA integrity_check').fetchall() != [('ok',)]:
        raise RuntimeError('SQLite integrity failed')
    database.row_factory = sqlite3.Row
    for table, count in snapshot['tableCounts'].items():
        # Only identifiers declared by the backup itself; safely quote them.
        quoted = '"' + table.replace('"', '""') + '"'
        rows = [dict(row) for row in database.execute(f'SELECT * FROM {quoted}')]
        if len(rows) != count:
            raise RuntimeError(f'Row count mismatch: {table}')
        key = lambda row: json.dumps(row, sort_keys=True, separators=(',', ':'))
        if sorted(map(key, rows)) != sorted(map(key, snapshot['tables'][table])):
            raise RuntimeError(f'Finance content mismatch: {table}')
print(f'Checksums, SQLite integrity and exact contents of {len(snapshot["tableCounts"])} finance tables verified.')
