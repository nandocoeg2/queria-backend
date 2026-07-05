use queria_backup::manifest::{BackupManifest, artifact_key, manifest_key, sha256_hex};
use queria_backup::object_store::ObjectStore;
use queria_backup::postgres::{backup_postgres, pg_dump_version};
use queria_backup::restore_drill::run_restore_drill;
use sqlx::postgres::PgPoolOptions;

#[tokio::test]
async fn test_object_store_lifecycle() {
    let endpoint = "http://127.0.0.1:17678";
    let bucket_name = "queria-test-bucket";
    let access_key = "queria";
    let secret_key = "queria-local-dev-only";
    let region = "us-east-1";

    let store = ObjectStore::new(endpoint, bucket_name, access_key, secret_key, region)
        .expect("should create ObjectStore");

    // Ensure the bucket exists (creates it if missing)
    store
        .ensure_bucket()
        .await
        .expect("ensure_bucket should succeed");

    // Put object
    let key = "test/integration/hello.txt";
    let data = b"hello integration tests";
    store
        .put_object(key, data, "text/plain")
        .await
        .expect("put_object should succeed");

    // Get object
    let retrieved = store
        .get_object(key)
        .await
        .expect("get_object should succeed");
    assert_eq!(retrieved, data);

    // List objects
    let objects = store
        .list_objects("test/integration/")
        .await
        .expect("list_objects should succeed");
    assert!(objects.iter().any(|obj| obj.key == key));

    // Delete object
    store
        .delete_object(key)
        .await
        .expect("delete_object should succeed");

    // Verify deleted
    let objects_after = store
        .list_objects("test/integration/")
        .await
        .expect("list_objects should succeed");
    assert!(!objects_after.iter().any(|obj| obj.key == key));
}

#[tokio::test]
async fn test_postgres_backup_and_drill() {
    let endpoint = "http://127.0.0.1:17678";
    let bucket_name = "queria-local"; // Use the default dev bucket
    let access_key = "queria";
    let secret_key = "queria-local-dev-only";
    let region = "us-east-1";

    let store = ObjectStore::new(endpoint, bucket_name, access_key, secret_key, region)
        .expect("should create ObjectStore");

    store
        .ensure_bucket()
        .await
        .expect("ensure_bucket should succeed");

    // Confirm pg_dump is available
    let pg_version = pg_dump_version()
        .await
        .expect("pg_dump should be functional");
    assert!(pg_version.contains("pg_dump"));

    // Connect to local test database
    let db_url = "postgres://queria:queria@127.0.0.1:17675/queria";
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(db_url)
        .await
        .expect("failed to connect to test db");

    queria_db::migrate::run_migrations(&pool)
        .await
        .expect("failed to run migrations");

    let org_slug = "fjulian";

    // 1. Run postgres backup
    let (pg_key, pg_checksum, pg_size) = backup_postgres(&store, db_url, org_slug)
        .await
        .expect("backup_postgres failed");

    assert!(pg_key.contains("fjulian/pg-dump/"));
    assert!(pg_size > 0);

    // 2. Mock a Qdrant snapshot (skip calling actual Qdrant to avoid strict schema setup requirements)
    // We can just verify it is recorded in the manifest
    let mut manifest = BackupManifest::new(org_slug, "20260705000100", "voyage-4-1024-v1");
    manifest.pg_dump_version = pg_version;
    manifest.pg_dump_key = pg_key.clone();
    manifest.add_checksum(&pg_key, &pg_checksum);
    let qdrant_key = artifact_key(org_slug, "qdrant-snapshot", "test.snapshot");
    let qdrant_snapshot = b"integration-test-snapshot";
    store
        .put_object(&qdrant_key, qdrant_snapshot, "application/octet-stream")
        .await
        .expect("failed to upload test Qdrant snapshot");
    manifest.qdrant_snapshot_key = Some(qdrant_key.clone());
    manifest.add_checksum(&qdrant_key, &sha256_hex(qdrant_snapshot));
    manifest.sign(secret_key);

    // Upload manifest
    let m_key = manifest_key(org_slug);
    let manifest_bytes = manifest.to_json_bytes();
    store
        .put_object(&m_key, &manifest_bytes, "application/json")
        .await
        .expect("failed to upload manifest");

    // 3. Run restore drill
    let drill = run_restore_drill(&store, org_slug, secret_key)
        .await
        .expect("run_restore_drill failed");

    assert!(drill.manifest_found);
    assert!(drill.pg_dump_exists);
    assert!(drill.pg_dump_checksum_ok);
    assert!(drill.all_passed);
    assert!(drill.errors.is_empty());
}

#[tokio::test]
async fn test_database_retention() {
    // Connect to local test database
    let db_url = "postgres://queria:queria@127.0.0.1:17675/queria";
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(db_url)
        .await
        .expect("failed to connect to test db");

    queria_db::migrate::run_migrations(&pool)
        .await
        .expect("failed to run migrations");

    // Insert a test organization
    let org_id = uuid::Uuid::now_v7();
    let org_slug = format!(
        "retention-{}",
        uuid::Uuid::now_v7().to_string()[..8].to_owned()
    );
    sqlx::query("INSERT INTO organization (id, slug, name) VALUES ($1, $2, $3)")
        .bind(org_id)
        .bind(&org_slug)
        .bind("Retention Test Org")
        .execute(&pool)
        .await
        .expect("failed to insert test org");

    // Insert audit log (created_at = 31 days ago)
    let audit_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO audit_log (id, organization_id, actor_type, action, resource_type, created_at)
         VALUES ($1, $2, 'test', 'test_action', 'test_resource', now() - interval '31 days')",
    )
    .bind(audit_id)
    .bind(org_id)
    .execute(&pool)
    .await
    .expect("failed to insert expired audit log");

    // Insert terminal ingestion job (updated_at = 31 days ago, status = succeeded)
    let job_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO ingestion_job (id, organization_id, status, job_type, updated_at, created_at)
         VALUES ($1, $2, 'succeeded', 'git_ingestion', now() - interval '31 days', now() - interval '31 days')"
    )
    .bind(job_id)
    .bind(org_id)
    .execute(&pool)
    .await
    .expect("failed to insert expired ingestion job");

    // Insert backup record (created_at = 31 days ago)
    let backup_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO backup_record (id, organization_id, backup_type, status, created_at)
         VALUES ($1, $2, 'full', 'succeeded', now() - interval '31 days')",
    )
    .bind(backup_id)
    .bind(org_id)
    .execute(&pool)
    .await
    .expect("failed to insert expired backup record");

    // Initialize mock object store
    let endpoint = "http://127.0.0.1:17678";
    let bucket_name = "queria-local";
    let access_key = "queria";
    let secret_key = "queria-local-dev-only";
    let region = "us-east-1";
    let store = ObjectStore::new(endpoint, bucket_name, access_key, secret_key, region).unwrap();

    // Run retention cleanup
    let report = queria_backup::retention::run_retention(&pool, &store, &org_slug, 30)
        .await
        .expect("run_retention failed");

    assert!(report.audit_logs_deleted >= 1);
    assert!(report.ingestion_jobs_deleted >= 1);
    assert!(report.backup_records_deleted >= 1);

    // Verify deleted from database
    let audit_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM audit_log WHERE id = $1)")
            .bind(audit_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!audit_exists);

    let job_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ingestion_job WHERE id = $1)")
            .bind(job_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!job_exists);

    let backup_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM backup_record WHERE id = $1)")
            .bind(backup_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!backup_exists);

    // Clean up org
    sqlx::query("DELETE FROM organization WHERE id = $1")
        .bind(org_id)
        .execute(&pool)
        .await
        .expect("failed to clean up test org");
}
