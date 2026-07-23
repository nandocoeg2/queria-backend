# Backup and Restore Runbook

This runbook documents the operational procedures for backing up and restoring
Queria's data stores: PostgreSQL and Qdrant vector database.

## Architecture Overview

```
┌─────────────┐     ┌──────────────┐     ┌───────────────┐
│  Worker      │────▶│  MinIO / S3  │◀────│  Restore CLI  │
│  (scheduled) │     │              │     │  (manual)     │
└──────┬───────┘     └──────────────┘     └───────────────┘
       │                    │
       ▼                    ▼
┌─────────────┐     ┌──────────────┐
│  PostgreSQL  │     │  Manifest    │
│  (pg_dump)   │     │  (checksums) │
├─────────────┤     └──────────────┘
│  Qdrant      │
│  (snapshot)  │
└─────────────┘
```

**Backup artifacts are stored in S3-compatible object storage** (MinIO locally,
OCI Object Storage in production) with the following key structure:

```
{org_slug}/pg-dump/{date}/queria_{timestamp}.dump
{org_slug}/qdrant-snapshot/{date}/{collection}_{timestamp}.snapshot
{org_slug}/manifests/{date}/manifest_{timestamp}.json
```

## Scheduled Backups

The worker process runs a daily backup at the configured hour
(`QUERIA_BACKUP_CRON_HOUR_UTC`, default: 02:00 UTC). Each backup:

1. Runs `pg_dump --format=custom` and uploads to S3.
2. Creates a Qdrant collection snapshot, downloads it, and uploads to S3.
3. Generates a manifest JSON with SHA-256 checksums for all artifacts.
4. Signs the manifest with the configured backup secret.
5. Records the backup in the `backup_record` table.
6. Runs retention cleanup (see below).

## Retention Policy

After each backup, the retention job removes data older than
`QUERIA_BACKUP_RETENTION_DAYS` (default: 30 days):

| Data Type | Retention | Notes |
|---|---|---|
| Audit logs | 30 days | All rows deleted |
| Terminal ingestion jobs | 30 days | succeeded/failed/cancelled only |
| Evaluation reports | 30 days | All rows deleted |
| Rejected/deprecated knowledge | 30 days | Approved items are **never** deleted |
| S3 backup artifacts | 30 days | All artifact types |
| Backup records | 30 days | DB tracking rows |

## Pre-Restore Checklist

Before starting a restore:

- [ ] Stop all Queria services (API, MCP, Worker, Proxy).
- [ ] Identify the manifest to restore from.
- [ ] Verify the manifest checksums (run restore drill first).
- [ ] Ensure the target PostgreSQL instance is empty or you accept data loss.
- [ ] Ensure the target Qdrant instance has no conflicting collections.

## Restore PostgreSQL

### Step 1: Download the dump from S3

```bash
# Using MinIO client (mc)
rtk mc alias set queria http://127.0.0.1:17678 queria queria-local-dev-only
rtk mc cp queria/queria-local/{org}/pg-dump/{date}/queria_{timestamp}.dump ./restore.dump

# Or using aws CLI
rtk aws s3 cp s3://queria-local/{org}/pg-dump/{date}/queria_{timestamp}.dump ./restore.dump \
  --endpoint-url http://127.0.0.1:17678
```

### Step 2: Verify checksum

```bash
rtk sha256sum ./restore.dump
# Compare with the value in the manifest JSON
```

### Step 3: Restore into PostgreSQL

```bash
# Drop and recreate the database
rtk psql -h 127.0.0.1 -p 17675 -U queria -c "DROP DATABASE IF EXISTS queria_restore;"
rtk psql -h 127.0.0.1 -p 17675 -U queria -c "CREATE DATABASE queria_restore;"

# Restore the dump
rtk pg_restore \
  --host=127.0.0.1 \
  --port=17675 \
  --username=queria \
  --dbname=queria_restore \
  --clean \
  --if-exists \
  --no-owner \
  --no-privileges \
  ./restore.dump

# Verify the restore
rtk psql -h 127.0.0.1 -p 17675 -U queria -d queria_restore \
  -c "SELECT version, name FROM _queria_migration ORDER BY version;"
```

### Step 4: Run migrations in verification mode

```bash
# Point the application at the restored database and run migrations
QUERIA_DATABASE_URL=postgres://queria:queria@127.0.0.1:17675/queria_restore \
  rtk cargo run -p queria-cli -- database migrate
```

## Restore Qdrant

### Option A: Restore from snapshot

```bash
# Download the snapshot
rtk mc cp queria/queria-local/{org}/qdrant-snapshot/{date}/{collection}_{timestamp}.snapshot \
  ./restore.snapshot

# Verify checksum
rtk sha256sum ./restore.snapshot
# Compare with manifest

# Upload and recover the snapshot
rtk curl -X POST \
  "http://127.0.0.1:17676/collections/{collection}/snapshots/upload?wait=true&priority=snapshot" \
  -H "api-key: ${QDRANT_API_KEY}" \
  -F "snapshot=@./restore.snapshot"
```

### Option B: Rebuild from PostgreSQL

If the Qdrant snapshot is unavailable, rebuild embeddings from PostgreSQL:

```bash
# 1. Drop the old collection
rtk curl -X DELETE "http://127.0.0.1:17676/collections/{collection}" \
  -H "api-key: ${QDRANT_API_KEY}"

# 2. Start the worker – it will recreate the collection and re-embed all chunks
rtk cargo run -p queria-worker
```

> **Note**: Option B requires the Voyage API key and will re-consume embedding
> API quota. Use Option A when possible.

## Post-Restore Verification

After restoring both PostgreSQL and Qdrant:

### 1. Run MCP doctor

```bash
rtk cargo run -p queria-cli -- doctor mcp --url http://127.0.0.1:17672/mcp
# Should return {"status": "ok"}
```

### 2. Run retrieval probe

```bash
rtk curl -X POST http://127.0.0.1:17671/api/v1/projects/{project_slug}/retrieval/probe \
  -H "Content-Type: application/json" \
  -d '{"query": "test query", "limit": 5, "include_global": true}'
# Should return relevant results
```

### 3. Run evaluation suite (CLI only)

```bash
rtk cargo run -p queria-cli -- eval run --project {project_slug}
# Verify scores match pre-restore baseline
```

## Restore Drill (ops only)

**Not part of default install or laptop onboarding.** Product pitch is hub TUI + Daily agent; restore-drill is a maintainer ops path. The CLI subcommand remains invocable but is **hidden from default `backup --help`** (Wave 2); use this runbook when you need the drill.

Run a read-only integrity drill:

```bash
# Hidden from product help; still parses for ops:
queria-cli backup restore-drill --org {org_slug}
# or from a workspace checkout:
# cargo run -p queria-cli -- backup restore-drill --org {org_slug}
```

Run an actual restore into empty PostgreSQL and Qdrant targets:

```bash
queria-cli backup restore-drill \
  --org {org_slug} \
  --target-database-url ****************************************/queria_restore \
  --target-qdrant-url http://127.0.0.1:17676 \
  --target-qdrant-collection {restore_collection}
```

The drill checks:
- ✅ Latest manifest exists in S3.
- ✅ Manifest signature matches.
- ✅ PostgreSQL dump file exists and checksum matches.
- ✅ Qdrant snapshot file exists and checksum matches.
- ✅ All checksums in the manifest are verified.

## Rollback

If a restore fails:

1. **Stop all services** immediately.
2. **Re-deploy from original volumes** (Docker volumes are not affected by
   restore operations if you used a separate database name).
3. **Investigate** the failure using the restore drill report.
4. If the backup itself is corrupt, fall back to the previous day's backup.

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `QUERIA_MINIO_ENDPOINT` | `http://127.0.0.1:17678` | S3-compatible endpoint |
| `QUERIA_MINIO_BUCKET` | `queria-local` | Bucket name |
| `QUERIA_MINIO_ACCESS_KEY` | `queria` | Access key |
| `QUERIA_MINIO_SECRET_KEY` | `queria-local-dev-only` | Secret key |
| `QUERIA_MINIO_REGION` | `us-east-1` | Region (arbitrary for MinIO) |
| `QUERIA_BACKUP_RETENTION_DAYS` | `30` | Days to keep artifacts |
| `QUERIA_BACKUP_CRON_HOUR_UTC` | `2` | Hour (UTC) to run daily backup |
| `QUERIA_SOURCE_COMMIT` | empty | Commit embedded in backup manifests; set this in CI |
