# Presto Presupuesto — project handover

Last updated: 2026-08-23

## Executive summary

Presto Presupuesto is an Expo 57 / React Native Android app for entering double-entry personal-finance transactions into an existing formula-rich Google spreadsheet. The spreadsheet remains the primary backing store. The app-side Google Sheets gateway is implemented and has been tested against the test workbook; production has not been touched.

The next immediate deliverable is a newly built, connected APK with the public OAuth web client ID and Apps Script deployment ID embedded. The latest Windows incremental Android build ended without producing an APK. Its live output was lost when the earlier task was interrupted, so there is no trustworthy final error transcript to diagnose. For repeatable future builds, GitHub Actions is the recommended route after stable APK signing is configured. WSL is a reasonable local-build alternative, but it requires a fresh Linux Android toolchain and native rebuild.

## Repository and workspace

- GitHub: <https://github.com/TimDumol/prestosupuesto>
- Active workspace: `E:\codex\prestosupuesto`
- Original working copy: `C:\Users\paloj\Documents\Codex\2026-08-20\i-have-a-google-spreadsheet-with\prestosupuesto`
- Temporary generated native project used by the current Windows build: `C:\p`
- Gradle cache: `E:\codex\prestosupuesto\.gradle-home`

The repository currently has uncommitted app, gateway, scripts, configuration, and documentation changes. Do not discard or reset them. Private configuration and generated files are ignored by Git.

The `E:` copy has a Windows ownership/ACL mismatch: ordinary sandboxed commands and `apply_patch` may fail before launch, while commands run as the signed-in Windows user work. This is one reason a clean clone in GitHub Actions or in WSL is preferable to continuing to repair this copy's permissions.

## Current application state

- Expo SDK 57 and React Native 0.86.2.
- Android application ID: `com.prestosupuesto.app`.
- The UX supports the finance workbook's double-column model:
  - date and description;
  - from/to category;
  - from/to account;
  - from/to currency and amounts;
  - expenses, income, transfers, and budget reallocations.
- Categories and accounts can be loaded dynamically from the sheet gateway.
- Recent transactions can be loaded from the gateway.
- Transaction submission supports cross-currency `toAmount` and UUID-based duplicate protection.
- A local fallback remains available when the gateway is not configured.
- Type checking and 13 unit tests passed after the integration work.

The previously generated APK is a UX/local-fallback build and is not the connected build:

`C:\Users\paloj\Documents\Codex\2026-08-20\i-have-a-google-spreadsheet-with\prestosupuesto\PrestoPresupuesto-ux-gateway-v1.1.0-arm64.apk`

Its SHA-256 is `921829CD0672AD083969948125BD59D96608439C6B97D0DF9A68DBE533154D78`.

## Google workbook contract and test state

- Test workbook ID: `1Jc7BwAK275iyvVGjjw_XQCLfDweowDtO8YGghgTdjdQ`.
- Production workbook was not modified.
- The test workbook has hidden transaction metadata columns:
  - `Transactions!R`: Transaction ID
  - `Transactions!S`: Created At
  - `Transactions!T`: Created By
- `Accounts!R` contains `IsActive` checkboxes.
- Re-running the migration in dry-run mode is idempotent.
- A direct test transaction was written successfully. Formula integrity, five validation rules, and metadata readback all passed.
- The gateway snapshot currently returns 3,023 transactions plus the workbook's budgets and accounts.
- Submitting an existing transaction UUID returned `duplicate: true` and did not add a row.

The detailed workbook assumptions and review material are in:

- `docs/sheet-understanding-review.md`
- `docs/workbook-contract.md`
- `docs/google-sheets.md`

## Apps Script gateway

- Bound Apps Script project ID: `1gFtRJL1MetI_jZpbVcWA0jam1H2nGPW4rDqQBh6eofZ_-DxpDmngd7XC`
- Google Cloud project number: `173607114902`
- Latest deployed script version: 4
- Latest deployment ID: `AKfycbynT_iU3Qu8VPGvo_pbR81YKB8ykFLjAVlJJ1lp8TE8CcyopPN--WlesSAhmhpjfhOY`
- Previously deployed version 3 ID, currently used by `.env.local`: `AKfycbyv55yFrAVn36amVPKinVB7DSPQArU0RfruvFOYasGchHLGm9IDervhuGt4jicgLJHF`

Version 3 was re-tested after version 4 was deployed and remains healthy. It identified the authorized caller, test workbook, and `Asia/Manila` workbook timezone correctly. Therefore the existing local environment remains usable; switching to version 4 is optional until its endpoint is explicitly smoke-tested.

The current owner allowlist maps `timdumol@gmail.com` to the app user label `td_prestosupuesto`. The wife's Google account has not yet been configured.

Relevant implementation and operating notes:

- `apps-script/`
- `scripts/`
- `docs/apps-script-gateway.md`
- `.google/apps-script-deployment.json`

## OAuth configuration

Private OAuth downloads are stored under `.google/` and are Git-ignored:

- `.google/gcp-android-client.json`
- `.google/gcp-web-client.json`

Validated public identifiers:

- Android OAuth client ID: `173607114902-9r451qok3qbvlmudgn9n1batet2vuire.apps.googleusercontent.com`
- Web OAuth client ID: `173607114902-gdvlt431o4i1kb8f7dg4qesh4h7hgoe1.apps.googleusercontent.com`
- Registered Android SHA-1: `5E:8F:16:06:2E:A3:CD:2C:4A:0D:54:78:76:BA:A6:F3:8C:AB:F6:25`

The web OAuth JSON includes a client secret. Never commit it, expose it in logs, or embed it in the APK. The web client ID and Apps Script deployment ID are public identifiers and may be embedded through Expo public environment variables.

`.env.local` is Git-ignored and currently contains the web client ID and the healthy version 3 deployment ID. It does not need the web client secret.

Before the wife can sign in:

1. If the OAuth consent screen is still in Testing, add her Google account as a test user.
2. Add her email and desired `createdBy` label to the gateway allowlist using the owner-only `setAllowedUsers` operation.

## Latest connected APK build attempt

The Windows build was started from the generated native project at `C:\p` with:

- the public web OAuth client ID;
- the healthy version 3 Apps Script deployment ID;
- Java 21;
- one Gradle worker and one native compile job;
- ARM64 only;
- Gradle cache on `E:` to avoid the nearly full `C:` drive.

The build initially remained active after the earlier task was interrupted, but the final handover check found no remaining Java or Node build processes and no output at:

`C:\p\android\app\build\outputs\apk\release\app-release.apk`

The attempt should therefore be treated as unsuccessful, not pending. Its final console output is unavailable. Before retrying locally, the generated native project and Gradle state can be inspected with:

```powershell
Get-Process java,node -ErrorAction SilentlyContinue
Test-Path 'C:\p\android\app\build\outputs\apk\release\app-release.apk'
```

After any successful retry, copy the APK into the repository under a descriptive ignored filename, calculate its SHA-256, inspect its signer certificate, install it on a test Android device, and verify:

1. Google sign-in succeeds.
2. The workbook snapshot loads categories, accounts, and recent transactions.
3. A harmless test transaction writes once.
4. Re-submitting the same transaction UUID is reported as a duplicate and does not add another row.

## Recommended build route: GitHub Actions

GitHub Actions is the best next build route because it is reproducible and avoids the current Windows filesystem, path-length, disk-space, and ACL issues. The existing workflow is `.github/workflows/android-apk.yml`, but it needs two changes before it can produce a login-capable APK reliably.

### 1. Supply public build configuration

Create GitHub repository variables and expose them to the prebuild/build steps:

- `EXPO_PUBLIC_GOOGLE_WEB_CLIENT_ID`
- `EXPO_PUBLIC_APPS_SCRIPT_DEPLOYMENT_ID`

Use the healthy version 3 deployment initially, or switch to version 4 after directly smoke-testing it.

### 2. Configure stable release signing

Google Android OAuth checks the package name and signing certificate SHA-1. A transient CI debug keystore will not match the registered SHA-1, so the APK may build while Google sign-in fails.

Preferred long-term approach:

1. Create a dedicated production release keystore.
2. Register that certificate's SHA-1 in a Google Android OAuth client for `com.prestosupuesto.app`.
3. Store the base64-encoded keystore and its passwords/aliases as encrypted GitHub Actions secrets.
4. Restore and use that same keystore for every release build.

For a short-lived smoke test, the existing debug key whose SHA-1 is already registered could be stored as a GitHub secret and used by CI. Do not commit a keystore or its passwords to the repository.

After those changes, push the current source, run the workflow, download the APK artifact, and execute the four-device smoke tests above.

## Alternative local route: WSL

WSL is suitable if local iteration is more important than hosted reproducibility. Clone into the Linux filesystem, for example `~/src/prestosupuesto`, rather than building under `/mnt/e`; Linux-native storage is faster and avoids Windows metadata/permission friction.

Required setup:

1. Install Node.js 22 and JDK 21.
2. Install Android SDK command-line tools, SDK 36, NDK `27.1.12297006`, and CMake `3.22.1`.
3. Run `npm ci`.
4. Create `.env.local` with only the public web client ID and Apps Script deployment ID.
5. Run `npx expo prebuild --platform android --non-interactive`.
6. Configure a stable signing keystore whose SHA-1 is registered for the Android OAuth client.
7. Build with a conservative native configuration:

```bash
cd android
./gradlew assembleRelease -PreactNativeArchitectures=arm64-v8a --max-workers=1
```

WSL requires downloading a complete Linux Android toolchain and rebuilding native dependencies, so it has more initial setup than GitHub Actions. It is the preferred fallback for repeatable local builds.

## Concrete next actions

1. Review the uncommitted changes and run `npm run typecheck` plus the test suite again.
2. Update GitHub Actions to inject the two public environment variables.
3. Choose and configure the permanent release signing key; register its SHA-1 with Google OAuth.
4. Commit and push the current source only after reviewing the changes and ensuring `.google/`, `.env.local`, keystores, Gradle caches, generated Android files, and APKs remain ignored.
5. Build the signed APK in GitHub Actions and run the device smoke tests.
6. Use a clean WSL clone as the fallback if GitHub Actions is unsuitable or local native debugging is needed.
7. Add the wife's OAuth test-user and gateway-allowlist entries.
8. After test-workbook sign-off, prepare a separately reviewed production-workbook configuration. Do not point the gateway at production casually.

## Security and safety reminders

- Never commit the web OAuth client secret, keystores, signing passwords, local tokens, or private sheet exports.
- Keep transaction UUIDs stable across retries so duplicate protection works.
- Continue testing writes against the test workbook until the full login/read/write flow is approved.
- Preserve workbook formulas and validation by using the gateway's established append/copy behavior rather than writing arbitrary ranges from the app.

