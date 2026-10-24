# Security review — 2026-10-24

Reviewed server authentication/session handling, OAuth and mobile PKCE, request-origin checks, SQL usage, Android network/auth configuration, dependency lockfiles, container isolation, and release configuration.

## Findings and fixes

- **Node.js / HTTP2Shell (CVE-2026-111497):** the local Node 24.21.0 runtime is affected. The production application is a Rust binary serving static JavaScript; its reqwest dependency has no Brotli feature or decoder in the resolved graph. Node is used for tooling and archive scripts. Require Node 24.22.1+ within 24.x via package engines and strict npm configuration, pin the Docker build and CI to 24.22.1, and reject older deployment runtimes. Use `nvm install` / `nvm use` with the committed `.nvmrc` to update local tooling. A patched runtime was installed under /tmp for this review; the workstation default was not replaced.
- **Vite:** update 7.3.6 to 7.3.7 for five advisories, including development-server command execution, cross-origin script execution, and filesystem allowlist bypass. The deployed service does not run the Vite development server.
- **Rust:** update ctutils to 0.4.5 (RUSTSEC-2026-0385); rustls to 0.23.46 (RUSTSEC-2026-0346 through 0349); rustls-webpki to 0.103.16 (RUSTSEC-2026-0382); and webpki-roots to 1.0.11 (RUSTSEC-2026-0381).
- **Additional Rust warnings:** update Tokio to 1.54.1 for RUSTSEC-2026-0386 and replace the yanked yoke-derive 0.8.3 with 0.8.4.
- **Release reproducibility:** build Rust with `--locked` and refresh Docker base images with `--pull` on deployment.

No additional exploitable application-code issue was established in the reviewed paths. Existing controls include parameterized database values, exact Pocket ID subject allowlisting, hashed session identifiers, single-use OAuth/mobile handoffs, mobile PKCE, same-origin checks for cookie writes, encrypted Google tokens, bounded request bodies, and a non-root read-only production container with dropped capabilities.

## Verification and limits

Both npm lockfiles and all 248 Rust dependencies pass their respective advisory audits after updates. Web unit tests, typechecking, production build, and server tests are run during release verification. Two database drill tests are intentionally ignored by the ordinary Rust suite and require the dedicated drill script. This is a source/dependency review, not a penetration test or an audit of every service on the shared host. Unrelated in-progress Android and push changes are excluded from this release.

Sources: [Node.js security release](https://nodejs.org/en/blog/vulnerability/october-2026-security-releases), [RustSec advisories](https://rustsec.org/advisories/), and npm audit results for the committed lockfiles.
