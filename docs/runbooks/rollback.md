# Production Rollback Runbook

> Status: CURRENT  
> Last verified: 2026-07-18  
> Deploy path: [`deployment.md`](./deployment.md)  
> Runtime truth: [`../HANDOFF.md`](../HANDOFF.md)

This runbook documents rolling back a failed production deployment **without wiping volumes**.

## Principles

- Prefer **rsync a known-good tree from the workstation** when host git is unavailable (common on this Oracle host).
- **Rebuild previous images on the host** (aarch64). Do not `docker volume rm` Postgres/Qdrant/object storage.
- Smoke on **host edge** `http://168.110.214.130:17674` (not domain-first).

## Identify the Rollback Version

Pick the last known-stable commit (or rsync snapshot) from release records / HANDOFF:

```bash
export ROLLBACK_COMMIT=deddf634a78fd5bc0cf2ad9e333bec65ceece5d1   # example only
```

## Rollback Procedure

### 1. Stop active app services (volumes kept)

```bash
ssh -i /Users/fernandojulian/project/knowledge-based-rag/ssh-key-2026-04-16.key ubuntu@168.110.214.130
cd /home/ubuntu/queria-backend
docker compose -f docker-compose.production.yml down
# Do NOT pass -v. No volume wipe.
```

### 2. Restore code tree

**Preferred (host git often broken):** from the **workstation**, rsync a known-good checkout:

```bash
# On workstation: ensure tree is at ROLLBACK_COMMIT first
cd /Users/fernandojulian/project/knowledge-based-rag/queria/backend
git checkout $ROLLBACK_COMMIT   # or worktree / archive of that commit

rsync -az --delete \
  --exclude '.git' \
  --exclude 'target' \
  --exclude 'node_modules' \
  --exclude '.env*' \
  ./ \
  ubuntu@168.110.214.130:/home/ubuntu/queria-backend/
```

Keep host secrets (`.env.production` / compose env) in place on the host; they are excluded from rsync.

**If host git works** (uncommon):

```bash
cd /home/ubuntu/queria-backend
git fetch --all
git checkout $ROLLBACK_COMMIT
```

### 3. Rebuild images on host (no registry assumption)

```bash
cd /home/ubuntu/queria-backend
export QUERIA_SOURCE_COMMIT=$ROLLBACK_COMMIT
docker compose -f docker-compose.production.yml build
```

Admin-only rollback (Astro only):

```bash
docker compose -f docker-compose.production.yml build queria-admin
docker compose -f docker-compose.production.yml up -d --no-deps queria-admin
```

### 4. Database compatibility

Migrations are **additive / backwards-compatible**. Do not run down-migrations. Optional verify:

```bash
docker compose -f docker-compose.production.yml run --rm --no-deps queria-api queria-cli database migrate
```

### 5. Start stable services

```bash
docker compose -f docker-compose.production.yml up -d
docker compose -f docker-compose.production.yml ps
```

### 6. Smoke on host:17674

```bash
curl -i http://168.110.214.130:17674/healthz
# open http://168.110.214.130:17674/admin/login
```

Domain `https://fjulian.id/...` is optional; 404 there does not by itself mean rollback failed if `:17674` is healthy.

## Related

| Doc | Use |
|---|---|
| [`deployment.md`](./deployment.md) | Primary rsync + host build path |
| [`../HANDOFF.md`](../HANDOFF.md) | Last known deployed commit / stack |
