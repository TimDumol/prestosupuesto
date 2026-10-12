# Authenticated visual tests

Run from the repository root:

```sh
npm --prefix web ci
npm exec --prefix web playwright install --with-deps chromium
npm run test:visual
```

Playwright builds the web app and serves it on localhost:4173. No Rust server,
Pocket ID account, database, environment secrets, or production network access
is needed. The session endpoint returns a synthetic Pocket ID session through
Playwright's test-only network routes. These tests cover the authenticated UI;
they do not verify the OAuth handshake or server authorization.

`finance.ts` contains synthetic API responses with decimal strings matching the
server wire format: eight budget categories (including overspending and
carryover), five accounts in EUR/USD/PHP (including credit card debt), salary,
expenses, a transfer, four months of charts, and a reconciliation preview with
post-closing activity. Date is frozen to 18 October 2026; timers continue to run.
Every test gets an isolated browser context. Unexpected API calls, mutations,
and uncaught JavaScript errors fail the test.

The suite captures all five pages, the transaction entry form, and a signed-out
control at 1440px and 390px widths. Baselines live beside the spec in
`authenticated.spec.ts-snapshots/` and should be committed with UI changes.
The signed-out control also checks that no finance endpoint is requested. Interaction checks cover localized amounts, transaction-effect previews, saved web drafts, repeated receipt entry, and responsive navigation without horizontal page overflow.

To review an intentional visual change:

```sh
npm run test:visual:update
npm --prefix web exec playwright show-report
```

Inspect the changed PNGs before committing them. Generate baselines on Linux
with the locked Playwright Chromium version, matching CI. Fonts and operating
systems affect screenshot rendering. CI runs on Ubuntu 24.04 and uploads its
HTML report, screenshots, diffs, and failure traces for 14 days. Keep the
Playwright lockfile and baselines in sync when upgrading the browser.

Screenshot assertions wait for stable consecutive frames and disable animation,
as described in the [Playwright visual comparison documentation](https://playwright.dev/docs/test-snapshots).

Readable typography is enforced in CI. All authored CSS and React/SVG font-size
values must use at least `1rem` (16px with the default browser preference), or
`var(--text-body)`. Smaller responsive overrides, pixel sizes, unknown size
variables, and font shorthand that hides a size fail the source lint. Use
`inherit` when a component should keep its parent's size. The lint parses CSS
with PostCSS and TS/TSX with TypeScript; it scans application source recursively
and excludes generated output, dependencies, and test code.

```sh
npm --prefix web run lint:typography     # source policy; no browser needed
npm --prefix web run test:accessibility # source policy + rendered text audit
```

`readability.spec.ts` checks the computed size of rendered text and form controls
on every main page and the login screen, at desktop and phone widths. It includes
SVG chart labels and tooltip portals, expanded funding/transaction details,
the month picker, and budget dropdown portals, and checks page overflow. It also verifies that budget mix labels,
percentages, and notes grow to 32px with a doubled root font preference. These
checks complement the screenshots and interaction tests; they do not certify
WCAG conformance or cover every possible UI state. Static lint cannot prove the
final cascade, inherited library styles, clipping, contrast, keyboard behavior,
or screen-reader semantics. The browser audit catches undersized text in its
covered states; visual review remains necessary for wrapping and overlap.

Our minimum size is a product readability requirement, not a WCAG minimum font
size. WCAG's [Resize Text requirement](https://www.w3.org/WAI/WCAG21/Understanding/resize-text)
requires support for enlargement up to 200% without loss of content or function.
General tools such as axe can complement this policy for contrast and semantic
accessibility checks; they do not replace the explicit minimum-size audit.
