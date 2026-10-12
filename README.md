# Presto Presupuesto

Personal finance with a Kotlin/Compose Android app, a React browser workspace,
and a Pocket ID-authenticated Rust API backed by canonical SQLite.
Google Sheets is the frozen archive and supports maintenance/import tooling.

## Android app

The Android client lives in [native-android/](native-android/README.md).
It supports expenses, salary/extra income/refunds, transfers, budget moves,
searchable activity, revision-guarded edits and corrections, budget history,
month-end balancing, offline writes, saved drafts and encrypted recovery archives.
Account reconciliation and administration are available in the browser workspace.

Build the APK with `npm run android:apk`, or build, install and launch with
`npm run android:apk -- --install`. The APK is copied to `PrestoNative.apk`.
Use Java 17 and Android SDK 36. The build script runs JVM tests and Android lint.
Run `npm run native:test -- --device --serial=SERIAL` for synthetic device tests.

The native Pocket ID callback is `prestosupuesto-native://pocket-id/callback`;
the server must support this callback before sign-in works.

## Development

Use Node.js 22.13 or newer for tooling, Rust stable for the server, and the Android
toolchain above. Run `npm ci` and `npm ci --prefix web` to install JavaScript tools.
Run `npm run lint`, `npm run typecheck`, `npm run test:web`, and `npm run web:build` for the web app. General lint includes the typography and browser accessibility checks.
Native GitHub Actions builds the APK and runs synthetic emulator tests.

## Browser analysis workspace

The standalone React workspace is built from `web/` with Vite and served by the
Rust/Axum application. The server authenticates Pocket ID sessions and reads and
writes canonical finance data in its persistent SQLite database. The native Android client connects to `https://athena.timdumol.com`. The Apps Script
maintenance gate remains closed after cutover; Google authorization is retained
for archive, rollback tooling and optional export work.

### Sign-in and OAuth setup

Every browser session begins with Pocket ID. The server only accepts the
two exact Pocket ID `sub` identifiers for the existing `tim` and `jopi`
accounts. `scripts/configure-pocket-id.sh` reads their stable user IDs from
Pocket ID and writes those values to the ignored server environment file.
Pocket ID sign-in is sufficient for canonical finance access; Google workbook
access is used by archive and rollback tools. See
the private cutover handoff (kept outside public version control) for production backup,
import parity and device recovery evidence.

Register the web OAuth callback in the Google Cloud project used by the Apps Script gateway:

```text
https://athena.timdumol.com/auth/google/callback
http://localhost:3000/auth/google/callback
```

Register and test the Pocket ID public client before deploying:

```bash
scripts/configure-pocket-id.sh
scripts/test-pocket-id-auth.sh
```

The setup script reads the admin key from the ignored `.pocket-id-api-key`
file, configures a PKCE client with localhost and Athena callbacks, resolves
the `tim` and `jopi` OIDC subjects, and never prints the key or subject IDs.
The test script starts a disposable localhost server and waits
for a real Pocket ID login by either `tim` or `jopi`. It must pass before the
deploy script will continue.

### Google OAuth callback

Use the existing Web application OAuth client.
Add the Athena and localhost callback URLs above to its **Authorized redirect
URIs** in Google Cloud Console.

The app requests the spreadsheet and email scopes. Both household accounts must
be in the Apps Script gateway allowlist and in the server's `ALLOWED_EMAILS`.

### Run locally

Install the standalone browser dependencies, build its static bundle, then run
the Rust server. No frontend dev server is needed. This requires Rust
stable and Node.js 22.13 or newer:

```bash
npm install --prefix web
npm run web:build
cp server.env.example .env.server.local
# Fill in OAuth IDs, secret, deployment ID, and the household email addresses.
# Generate TOKEN_ENCRYPTION_KEY once with: openssl rand -base64 32
set -a
. ./.env.server.local
set +a
npm run web
```

Open `http://localhost:3000`. Run `npm run web:typecheck` to type-check the
standalone React app. Keep the encryption key stable: SQLite refresh tokens
are encrypted with it and cannot be read after the key is changed.

### Host with Docker

Build and run the included single-container deployment behind an HTTPS reverse
proxy or a hosting platform that provides HTTPS. Mount a persistent volume at
`/app/data` so SQLite sessions and OAuth credentials survive restarts. Supply
the same environment variables above, with `PUBLIC_ORIGIN` and
`GOOGLE_REDIRECT_URI` set to the hosted HTTPS origin; keep `COOKIE_SECURE=true`.

```bash
docker build -t presto-finance .
docker run --restart unless-stopped -p 3000:3000 \
  -v presto-finance-data:/app/data \
  --env-file /secure/path/presto-finance.env \
  presto-finance
```

The environment file must stay outside Git. Do not place OAuth secrets or the
token-encryption key in the React build environment. Back up the SQLite volume
and retain the encryption key separately in the hosting provider's secret store.

### Deploy to Athena

The release script builds the image locally, streams it to the same Docker
host used by Aprendiendo, then uses Ansible to update `/opt/presto-finance`,
back up the SQLite database, add the `athena.timdumol.com` Caddy site, and check
`/healthz`. It requires a successful Pocket ID localhost test first:

```bash
scripts/deploy.sh
```

Keep `.env.server.local` and `.pocket-id-api-key` private. The deployment
script creates a production environment file in a mode-0600 temporary
directory and removes it when the release ends.

## Web visual tests

Run `npm run test:visual` for authenticated desktop and phone screenshots with
synthetic household fixtures. Setup, baseline updates, and CI details are in
[the visual test guide](web/visual/README.md).

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

Budget lifecycle and calculation rules: [calculated budget model](docs/budget-model.md).
