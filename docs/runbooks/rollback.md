# Production Rollback Runbook

This runbook documents the process for rolling back a failed production deployment.

## Identify the Rollback Version

Identify the last known stable Docker image tag or git commit hash from your release records or deployment logs:

```bash
# Example: switch to last stable commit
export ROLLBACK_COMMIT=6640b7b
```

## Rollback Procedure

To roll back the deployment without losing persistent database or vector volumes:

1. **Stop active services**:
   ```bash
   docker compose -f docker-compose.production.yml down
   ```

2. **Checkout the rollback commit**:
   ```bash
   git checkout $ROLLBACK_COMMIT
   ```

3. **Re-build or pull the rollback images**:
   If images are stored in a registry, update the image tag in `docker-compose.production.yml` and pull:
   ```bash
   docker compose -f docker-compose.production.yml pull
   ```
   If building locally on the host:
   ```bash
   docker build --build-arg QUERIA_SOURCE_COMMIT=$ROLLBACK_COMMIT -t queria-backend:latest .
   docker compose -f docker-compose.production.yml build queria-admin
   ```

4. **Verify Database Compatibility**:
   Since Queria database schema migrations are backwards-compatible and additive, no down-migrations are required. However, you can verify schema compatibility by running:
   ```bash
   # Check active database schema matches expectations
   docker compose -f docker-compose.production.yml run --rm queria-api database migrate
   ```

5. **Start the stable services**:
   ```bash
   docker compose -f docker-compose.production.yml up -d
   ```

6. **Verify health status**:
   Ensure all containers are healthy:
   ```bash
   docker compose -f docker-compose.production.yml ps
   ```
   Perform a smoke test:
   ```bash
   curl -i https://fjulian.id/healthz
   ```
