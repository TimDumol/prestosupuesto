# Canonical database: Phase D disposable drills

The Phase D transaction, reconciliation, month-close, and local SQLite recovery
drills pass against a fresh import of the checked-in sanitized fixture. The
drills run through the finance API handler functions with a Pocket session
inserted only into the temporary database; they do not contact Pocket ID or
Google Sheets. A synthetic ECB EUR/USD observation is added to that temporary
database so the USD reconciliation valuation path is exercised.

Run the complete drill with:

```sh
bash scripts/test-canonical-db-drill.sh
```

The script creates a mode-0700 directory under `/tmp`, imports and verifies the
fixture, runs the explicit ignored Rust drill, exports the mutated database,
restores to a new SQLite target with the export digest, and compares all table
rows and counts. It removes the temporary directory on exit.

The API drill covers:

- Pocket-session authorization and unauthenticated rejection;
- transaction create, exact retry, operation-ID conflict, financial edit,
  exact retry, and stale transaction revision rejection;
- partial reconciliation, rejection of closing with incomplete month-end
  coverage, full reconciliation, exact retry, replacement with stable
  reconciliation transaction IDs, and stale ledger revision rejection;
- month-close digest, exact retry, second-close conflict, closed-snapshot update
  rejection, and refusal to post into a closed month;
- SQLite integrity and foreign-key checks, followed by exact equality of the
  exported finance table contents after fresh-target restore.

The final run restored all 23 exported finance tables and compared all 385 rows.
An earlier run exposed missing entries in the restore command's post-restore
row-count verifier; that verifier now covers the full export table set, and the
fresh rerun passed. The source import had zero validation issues and zero
review warnings before the mutation drill.

## Physical Android phone drill

One connected Android phone completed an end-to-end snapshot recovery drill
against disposable databases. The test build was installed over the existing
package with the same signing certificate, then the original APK was restored
in place after cleanup. A synthetic local Pocket session authorized real HTTP
requests to the disposable backup and restore endpoints; live Pocket ID login
was not part of this drill.
