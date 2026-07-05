# Production Deployment Runbook

This runbook documents the deployment process for Queria's production environment.

## Pre-flight Host Verification

Before starting deployment, verify that the host meets the following requirements:

- **OS**: Ubuntu 22.04 LTS (recommended)
- **RAM**: Minimum 12 GB
- **CPU**: Minimum 2 Cores
- **Disk**: Minimum 190 GB free space
- **Docker**: Docker Engine and Compose plugin installed and active
- **DNS**: DNS records configured for the public domain (e.g., `fjulian.id`)
- **Port**: Port 443 open in host firewall

Run local pre-flight checks:

```bash
# Check RAM & CPU
free -h
nproc

# Check disk space
df -h /

# Check Docker status
systemctl status docker
```

## Secrets Injection (Infisical)

Production secrets are managed via Infisical. Before deploying, ensure the client machine is authenticated with Infisical and can fetch production secrets:

```bash
rtk infisical login
rtk infisical export --env=prod --format=dotenv > .env.production
```

Verify that `.env.production` contains all necessary keys without committing it:
- `DATABASE_PASSWORD`
- `QDRANT_API_KEY`
- `VOYAGE_API_KEY`
- `QUERIA_SETUP_TOKEN`
- `OCI_STORAGE_ENDPOINT`
- `OCI_STORAGE_BUCKET`
- `OCI_STORAGE_ACCESS_KEY`
- `OCI_STORAGE_SECRET_KEY`
- `OCI_STORAGE_REGION`

## Build and Package

Build the Docker images locally or in CI, tagging them with the target commit:

```bash
# Export the current commit hash
export QUERIA_SOURCE_COMMIT=$(git rev-parse HEAD)

# Build the Rust workspace Docker image
docker build --build-arg QUERIA_SOURCE_COMMIT=$QUERIA_SOURCE_COMMIT -t queria-backend:latest .

# Build the Astro Admin UI Docker image
docker compose -f docker-compose.production.yml build queria-admin
```

## Database Migration

Run database migrations once before booting the application containers to prevent concurrent migration attempts:

```bash
docker compose -f docker-compose.production.yml run --rm queria-api database migrate
```

## Deploying the Stack

Start all services in detached mode:

```bash
docker compose -f docker-compose.production.yml up -d
```

Verify that all services are running and healthy:

```bash
docker compose -f docker-compose.production.yml ps
```

## Verification Checks

After deployment, perform verification checks:

1. **Proxy Health Check**:
   ```bash
   curl -i https://fjulian.id/healthz
   # Should return 200 OK
   ```
2. **API Access**:
   ```bash
   curl -i https://fjulian.id/api/v1/healthz
   ```
3. **Admin UI Access**:
   Access the dashboard at `https://fjulian.id/admin/` and verify the page loads correctly.
