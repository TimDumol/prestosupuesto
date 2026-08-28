# Presto Presupuesto

React Native app for entering double-column personal-finance transactions and reallocating accumulated monthly budgets, with a test-only Google Sheets integration.

## Included

- Expense entry with a fixed budget-category picker and implicit `category → Expense` flow.
- Income entry with implicit `Income → category` flow.
- Account-to-account transfers with no budget movement.
- Category-to-category budget reallocations with no account movement.
- Accumulated monthly budgets, rollover, spending, and local reallocation feedback.
- Recent transaction history that exposes the underlying accounting flow.
- Google sign-in and an owner-controlled Apps Script gateway for the test workbook.
- Formula-preserving, serialized writes with UUIDv7 retry deduplication.

Without build-time Google identifiers the app remains a local demo. A configured build reads categories, active accounts, budgets, and recent transactions from the test workbook and submits new rows through the gateway. Offline persistence and biometrics are not implemented yet.

## Development

Requirements: Node.js 22.13 or newer, Java 21, and the Android SDK when building locally.

```bash
npm ci
npm run typecheck
npm start
```

Generate the native Android project:

```bash
npm run prebuild:android
```

On Windows, build the installable release APK with:

```powershell
npm run apk:release:windows
```

The APK is written to `android/app/build/outputs/apk/release/app-release.apk`.

The included GitHub Actions workflow also builds and uploads an APK artifact on pushes to `main` or manual runs.

## Google Sheets test tools

The repository includes credential-safe local scripts for authenticating, inspecting,
reading, appending, and patching rows in a Google Sheet. Writes are dry runs unless
you explicitly pass `--commit`.

See [docs/google-sheets.md](docs/google-sheets.md) for setup and examples, and
[docs/workbook-contract.md](docs/workbook-contract.md) for the sanitized schema
and transaction mappings discovered from the test copy. The owner-facing
[spreadsheet understanding review](docs/sheet-understanding-review.md) separates
confirmed behavior from assumptions that need approval before production writes.
See [docs/apps-script-gateway.md](docs/apps-script-gateway.md) for gateway deployment,
household access, OAuth client, and APK configuration.
