# Presto Presupuesto

React Native UX prototype for entering double-column personal-finance transactions and reallocating accumulated monthly budgets.

## Included

- Expense entry with a fixed budget-category picker and implicit `category → Expense` flow.
- Income entry with implicit `Income → category` flow.
- Account-to-account transfers with no budget movement.
- Category-to-category budget reallocations with no account movement.
- Accumulated monthly budgets, rollover, spending, and local reallocation feedback.
- Recent transaction history that exposes the underlying accounting flow.
- UX-only setup screen describing the future Google Sheets mapping.

All data is currently in memory and resets when the app closes. Google Sheets authentication, sync, offline persistence, and biometrics are intentionally not implemented yet.

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
