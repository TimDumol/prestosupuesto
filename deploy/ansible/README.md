# Athena deployment

`site.yml` deploys the Rust finance server and React bundle to the Docker host
shared with Aprendiendo, and adds `athena.timdumol.com` to its existing Caddy
site imports. The finance container has no published host port; it joins the
dedicated external bridge and persists SQLite under `/opt/presto-finance/data`.

Run `scripts/configure-pocket-id.sh` and complete `scripts/test-pocket-id-auth.sh`
before `scripts/deploy.sh`. The release script will refuse deployment without a
successful local Pocket ID callback for `tim` or `jopi`. Its production
environment file exists only in a mode-0600 temporary directory, and Ansible
suppresses task output while installing it on the host.

Copy `inventory/hosts.example.yml` to the Git-ignored `inventory/hosts.yml`
and set your host, SSH user and key path. Set the matching `PRESTO_SSH_HOST`,
`PRESTO_SSH_USER` and `PRESTO_SSH_KEY` variables in the Git-ignored
`.env.deploy.local` at the repository root. Deployment and laptop backup scripts
load that file automatically. Docker images are built as `linux/amd64` and
streamed directly to the host without a registry.

Before deploying, provision the dedicated Docker bridge and attach the shared
Caddy container. In the private inventory, set the bridge name, the finance
container's reserved IPv4 address, the HTTPS issuer hostname, and Caddy's IPv4
address on that bridge. Finance resolves the issuer through Caddy and exposes
its app port only on the isolated bridge. If using the shared WAF, set
`presto_caddy_waf_file` to the snippet file's path inside Caddy and ensure the
main Caddyfile imports it. The playbook checks these dependencies before
writing deployment files.

Manage host firewall rules, network provisioning, and WAF rollout separately
from application deployment. Keep the production topology and migration and
rollback runbooks in private operational storage. `deploy/private/` is ignored
for local copies.

Run the template regression tests with `python3 deploy/ansible/test-isolation.py`
using Jinja2 and PyYAML.
