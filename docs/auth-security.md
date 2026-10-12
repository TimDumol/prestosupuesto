# Authentication and session lifecycle

Presto issues its own opaque, fixed-lifetime sessions after Pocket ID login.
Only SHA-256 session hashes are stored on the server. Browser cookies are
HttpOnly and SameSite=Lax, and Secure in hosted configurations. Every protected
request checks the session expiry and the configured household subject allowlist.
A Presto session lasts 30 days; this is independent of the identity provider's
browser session. Pocket ID logout or account changes do not automatically revoke
an already issued Presto session. Use Presto logout to revoke that session, or
remove the subject from the server allowlist and restart the server to deny all
of that subject's sessions. Provider-driven immediate revocation would require
an additional back-channel logout integration.

Pocket ID authorization callbacks must match a short-lived state cookie in the
browser that initiated login. State expires after 10 minutes and is single-use.
Successful browser reauthentication revokes the browser's previous session.
Logout also cancels its pending login and deletes linked Google sessions and
Google authorization states. Pending Pocket ID state remains cancellable during
provider requests; state consumption, credential issuance and replacement-session
revocation commit together. A logout that cancels the state before issuance
prevents the delayed callback from issuing a browser session or mobile handoff.
Google callbacks from a different Presto session cannot consume the initiating
session’s authorization state. A delayed Google callback or token refresh cannot
recreate a session that has already expired or been revoked.

Mobile login additionally uses an app-generated S256 challenge. The app keeps the
verifier privately and sends it only in the handoff exchange POST. The 90-second
handoff is single-use and can only be exchanged with the matching verifier.
Invalid proof does not consume a legitimate handoff. This protects against
custom-scheme callback interception; a failed callback also retains the app's
proof so that the legitimate callback can finish.

Cookie-authenticated writes require the exact configured Origin. Mobile writes
without an Origin require a valid bearer-token format and authenticate using
that bearer alone; an invalid bearer never falls back to a browser cookie.
All API and auth responses use no-store. Responses prohibit framing, disable
object embedding, restrict base URLs, suppress referrers and disable MIME sniffing.

The browser clears authenticated finance state on 401, discards earlier responses
still in flight, and checks session validity every minute while visible and when
returning to the tab. Network errors do not imply expiry. Saved drafts remain.
Android clears locally expired credentials even offline, preserves queued entries,
and reports failed server logout without pretending revocation succeeded.

## Upgrade and verification

Migration `0015_mobile_handoff_pkce.sql` invalidates pre-upgrade pending mobile
logins and handoffs. New mobile logins require the updated Android client; existing
bearer sessions are unaffected until logout or expiry. Update the Android client
alongside the server. The auth-only migration leaves the finance table set
unchanged, but raises the interchange schema to 15. Updated Android recovery
validation accepts this version. Schema-14 recovery archives still require a
matching server schema to restore.

Regression coverage includes browser binding, handoff proof/expiry/replay,
allowlist removal, bearer/cookie mixing, cookie-write origin checks, logout cleanup,
delayed Google callbacks, Google outages, browser expiry and retained drafts.
Android instrumented regressions cover offline expiry and failed logout retries.

## Security review (2026-10-04)

Reviewed the server route authorization checks, provider discovery and code
exchange, browser cookies, bearer precedence, CSRF protection, session expiry and
revocation, mobile handoff proof, Android credential storage and callback handling,
web cache invalidation, and repository deployment configuration.

Three issues were corrected:

- A Pocket ID callback previously consumed its login state before making provider
  requests. Logout during those requests could not cancel the callback, allowing
  a fresh session after logout. State cancellation and credential issuance now
  serialize through SQLite transactions. Regression tests cover cancellation,
  expiry and replay for both browser sessions and mobile handoffs.
- A Google callback with another browser's state previously deleted that state
  before checking the initiating session. It could disrupt a legitimate login,
  although it could not authenticate the wrong session. The consuming query now
  checks the session binding before deletion, with a regression test.

- Browser logout previously chose a bearer header ahead of its Pocket ID cookie.
  A mixed request could clear the browser cookie without revoking its server
  session. Browser logout now revokes the cookie session regardless of bearer
  headers; mobile logout continues to use bearer authentication. Regression tests
  cover invalid and unrelated bearer headers alongside a valid cookie.

No additional authentication bypass was found in the reviewed paths. This is a
source review with local regression tests, not verification of the deployed
Pocket ID client settings, Google consent settings or live reverse proxy.
The Pocket ID flow uses the access token returned directly by the trusted HTTPS
provider to fetch its UserInfo subject; it does not validate an ID token. Its
security depends on the pinned provider enforcing client/redirect binding and
S256 PKCE. The PKCE checks were reviewed against
[RFC 9700](https://www.rfc-editor.org/rfc/rfc9700.html).

The workspace intentionally grants every allowed subject access to the same
household finance data, including backup and enabled restore operations. Google
credentials have a separate email allowlist and are linked to the issuing Presto
session. This model does not provide per-user finance isolation or an admin role.
Provider logout alone does not revoke the 30-day Presto session, as described
above. Requests already authorized before logout may finish; logout is not a
rollback of in-flight finance operations.

Verification: server tests passed (31 passed, two finance drills intentionally
ignored), all 37 web unit tests passed, and all four browser authentication
expiry tests passed across desktop and phone viewports. Android authentication
code and existing regression coverage were inspected; device instrumentation
and live identity-provider integration were not run during this review.
