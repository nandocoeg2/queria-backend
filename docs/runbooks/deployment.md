# Production Deployment Runbook

> Status: CURRENT  
> Last verified: 2026-07-18  
> Runtime truth: [`../HANDOFF.md`](../HANDOFF.md)

This runbook documents the deployment process for Queria's production environment.

## Production Host Access

| Field | Value |
|---|---|
| Host | `168.110.214.130` |
| User | `ubuntu` |
| Hostname | `instance-20260518-2039` (Oracle Cloud aarch64) |
| Deploy directory | `/home/ubuntu/queria-backend` |
| SSH key (local workspace) | `ssh-key-2026-04-16.key` + `ssh-key-2026-04-16.key.pub` |

```bash
ssh -i /Users/fernandojulian/project/knowledge-based-rag/ssh-key-2026-04-16.key ubuntu@168.110.214.130
cd /home/ubuntu/queria-backend
```

Do not commit private keys. Prefer agent-forwarding or a secrets manager for shared operator access.

Notes from live host (verified 2026-07-18):

- Queria stack runs via `docker-compose.production.yml` under `/home/ubuntu/queria-backend`.
- Public edge is Caddy (`queria-edge`) on host port `17674` (host Nginx already owns 80/443 for other sites). Path routing lives in `docker/Caddyfile`.
- Shared host: monitoring, other Postgres app stacks, and non-Queria containers coexist.
- **Primary deploy path is manual rsync + build on host** — not git-push CI, and not host `git fetch`/`git pull` (GitHub SSH from the host is often broken).

## Pre-flight Host Verification

Before starting deployment, verify that the host meets the following requirements:

- **OS**: Ubuntu 22.04/24.04 LTS (current host is Ubuntu 24.04 Oracle aarch64)
- **RAM**: Minimum 12 GB
- **CPU**: Minimum 2 Cores
- **Disk**: Minimum 190 GB free space
- **Docker**: Docker Engine and Compose plugin installed and active
- **DNS** (optional): domain `fjulian.id` only if Nginx is wired to the edge; may still 404 — prefer host `:17674` smoke below
- **Port**: Primary public MVP path is host port **`17674`** (Caddy edge). Port `443` via Nginx is optional and may not route to Queria

Run local pre-flight checks on the host:

```bash
# Check RAM & CPU
free -h
nproc

# Check disk space
df -h /

# Check Docker status
systemctl status docker
docker ps --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}"
```

## Secrets Injection (Infisical)

Production secrets are managed via Infisical. Before deploying, ensure the client machine is authenticated with Infisical and can fetch production secrets:

```bash
rtk infisical login
rtk infisical export --env=prod --format=dotenv > .env.production
```

Copy `.env.production` to the host deploy directory (never commit it). Verify it contains the keys your stack needs (names vary by env setup; do not invent or paste secret values here), commonly including:

- `DATABASE_PASSWORD`
- `QDRANT_API_KEY`
- `VOYAGE_API_KEY`
- `QUERIA_SETUP_TOKEN`
- `OCI_STORAGE_ENDPOINT`
- `OCI_STORAGE_BUCKET`
- `OCI_STORAGE_ACCESS_KEY`
- `OCI_STORAGE_SECRET_KEY`
- `OCI_STORAGE_REGION`

## Primary deploy: rsync → build on host (aarch64)

Do **not** rely on CI push or on-host git checkout when GitHub access is broken. Sync a known-good tree from the workstation, then build for the host architecture.

From the **workstation** (workspace root or `queria/backend` as appropriate):

```bash
# Example: sync backend tree to host deploy dir (adjust excludes as needed)
rsync -az --delete \
  --exclude '.git' \
  --exclude 'target' \
  --exclude 'node_modules' \
  --exclude '.env*' \
  /Users/fernandojulian/project/knowledge-based-rag/queria/backend/ \
  ubuntu@168.110.214.130:/home/ubuntu/queria-backend/
```

Ensure secrets (`.env.production` or compose env files) are present on the host separately — rsync above excludes `.env*`.

On the **host**:

```bash
cd /home/ubuntu/queria-backend
export QUERIA_SOURCE_COMMIT=$(git rev-parse HEAD 2>/dev/null || echo "rsync-$(date +%Y%m%d)")

# Build production images on aarch64 host
docker compose -f docker-compose.production.yml build

# Migrations once before boot (entrypoint wraps binaries)
docker compose -f docker-compose.production.yml run --rm --no-deps queria-api queria-cli database migrate

# Start stack
docker compose -f docker-compose.production.yml up -d

docker compose -f docker-compose.production.yml ps
```

### Admin-only rebuild (Astro changes only)

When only `queria-admin` changed, avoid full stack rebuild:

```bash
cd /home/ubuntu/queria-backend
docker compose -f docker-compose.production.yml build queria-admin
docker compose -f docker-compose.production.yml up -d --no-deps queria-admin
```

## Optional: local pre-build / package notes

If you build images locally for transfer (less common than host build on aarch64):

```bash
export QUERIA_SOURCE_COMMIT=$(git rev-parse HEAD)
docker build --build-arg QUERIA_SOURCE_COMMIT=$QUERIA_SOURCE_COMMIT -t queria-backend:latest .
docker compose -f docker-compose.production.yml build queria-admin
```

Prefer **build on host** so image arch matches Oracle aarch64.

## Verification Checks (primary smoke)

After deployment, use the **host edge** first:

1. **Health**:
   ```bash
   curl -i http://168.110.214.130:17674/healthz
   # expect 200 OK
   ```
2. **Admin login**:
   Open `http://168.110.214.130:17674/admin/login` and confirm the page loads.
3. **Compose status**:
   ```bash
   docker compose -f docker-compose.production.yml ps
   ```

### Domain (optional)

If Nginx is reverse-proxied for `fjulian.id` → edge, you may also try:

```bash
curl -i https://fjulian.id/healthz
```

Treat domain/path failures as **proxy/DNS config**, not necessarily a broken stack — host `:17674` is the authoritative smoke until Nginx is confirmed wired.

## Related

| Doc | Use |
|---|---|
| [`../HANDOFF.md`](../HANDOFF.md) | Deployed commit, stack identity |
| [`rollback.md`](./rollback.md) | Rsync known-good + rebuild without volume wipe |
| [`onboarding.md`](./onboarding.md) | Post-deploy admin/agent path |
| [`local-development.md`](./local-development.md) | Local compose (ports/env) |
