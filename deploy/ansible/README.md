# Athena deployment

`site.yml` deploys the Rust finance server and React bundle to the Docker host
shared with Aprendiendo, and adds `athena.timdumol.com` to its existing Caddy
site imports. The finance container has no published host port; it joins the
shared `caddy-edge` network and persists SQLite under `/opt/presto-finance/data`.

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
