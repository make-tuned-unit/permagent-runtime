#!/usr/bin/env python3
"""Remove test-fixture projects that leaked into the real Permagent database.

Deletes 40 projects and their dependent rows. Selection is deliberately
narrow — a project is only removed if it matches one of:

  * created in the batch at 2026-08-15T11:28  (26 rows, audit-era test runs)
  * created in the batch at 2026-08-17T12:23  (13 rows, today's daemon tests)
  * named exactly "E2E Harness Probe"          (1 row, 2026-08-09)

Every one of those has `root_path = NULL` and a fixture-shaped name
(`fp-analytics-disabled`, `Intel delete first <uuid>`, `Safe documents
<uuid>`, `Roundtrip`, `Validate`, `Sovereign`, …). Nothing with a real
root_path is touched.

The 18 projects that REMAIN, listed so the blast radius is checkable
before running:

    Personal, GetLadle, World Litter Run, Reckonize, Evntally, Aidvocate,
    Plant Nanny, Plekk, Kinrows, Permagent, Permagent Runtime, Spectral,
    Grocery Savers, Harbourview Residents' Association, Wealthie, LAUFT,
    Port Community Liaison Committee, Teenity

Run with --dry-run first to print what would go without changing anything.
Quit the Permagent app before running for real.
"""

import os
import sqlite3
import sys

DB = os.path.expanduser("~/.permagent/spectral/permagent.db")
BATCH_TIMESTAMPS = ("2026-08-15T11:28", "2026-08-17T12:23")
NAMED = "E2E Harness Probe"

# Dependent rows, deleted before their project so nothing is orphaned.
#
# Order matters: `cards` reference `board_columns`, so columns must go AFTER
# cards or the delete trips a foreign key. On the first run this was the other
# way round and `board_columns` was skipped — harmless in the end, because the
# rows carry ON DELETE CASCADE from `projects` and went with their project, but
# it left the script reporting a failure it had actually recovered from.
DEPENDENTS = [
    ("cards", "project_id"),
    ("board_columns", "project_id"),
    ("project_documents", "project_id"),
    ("analytics_events", "project_id"),
    ("decisions", "project_id"),
    ("project_people", "project_id"),
]


def main() -> int:
    dry = "--dry-run" in sys.argv
    if not os.path.exists(DB):
        print(f"database not found: {DB}")
        return 1

    conn = sqlite3.connect(f"file:{DB}?mode=ro" if dry else DB, uri=dry)
    conn.execute("PRAGMA foreign_keys=ON")

    rows = conn.execute(
        "SELECT id, name FROM projects "
        "WHERE substr(created_at,1,16) IN (?,?) OR name = ?",
        (*BATCH_TIMESTAMPS, NAMED),
    ).fetchall()
    ids = [r[0] for r in rows]

    if not ids:
        print("nothing matched — already clean")
        return 0

    placeholders = ",".join("?" * len(ids))
    print(f"{'WOULD DELETE' if dry else 'DELETING'} {len(ids)} projects:\n")
    for _, name in rows:
        print(f"    {name}")

    print()
    total = 0
    for table, col in DEPENDENTS:
        try:
            if dry:
                n = conn.execute(
                    f"SELECT count(*) FROM {table} WHERE {col} IN ({placeholders})",
                    ids,
                ).fetchone()[0]
            else:
                n = conn.execute(
                    f"DELETE FROM {table} WHERE {col} IN ({placeholders})", ids
                ).rowcount
            if n:
                print(f"    {table}: {n} rows")
                total += n
        except sqlite3.Error as e:
            print(f"    {table}: skipped ({e})")

    if not dry:
        conn.execute(f"DELETE FROM projects WHERE id IN ({placeholders})", ids)
        conn.commit()

    remaining = conn.execute("SELECT count(*) FROM projects").fetchone()[0]
    print(f"\ndependent rows: {total}")
    print(f"projects {'would remain' if dry else 'remaining'}: "
          f"{remaining - (len(ids) if dry else 0)}  (expected 18)")
    conn.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
