//! Multi-org hard isolation leakage tests (VAL-ISOL-*, VAL-CROSS-*).
//!
//! Fixtures create two orgs (A/B), members, projects, knowledge, tokens.
//! Session A must never observe B; super-admin without membership is denied on tenant routes.

#[cfg(test)]
mod tests {
    use crate::app::build_app_with_pool;
    use axum::body::{Body, to_bytes};
    use http::{Request, StatusCode as HttpStatus, header};
    use queria_core::AppConfig;
    use queria_core::auth::password::PasswordHasher;
    use serde_json::Value;
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    async fn test_pool() -> Option<PgPool> {
        let url = std::env::var("QUERIA_DATABASE_URL").ok()?;
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .ok()
    }

    async fn seed_member(
        pool: &PgPool,
        email: &str,
        password: &str,
        org_slug: &str,
        org_name: &str,
        is_platform_super_admin: bool,
    ) -> (Uuid, Uuid) {
        let org_id: Uuid = sqlx::query_scalar(
            "insert into organization(slug, name) values ($1, $2)
             on conflict (slug) do update set name = excluded.name
             returning id",
        )
        .bind(org_slug)
        .bind(org_name)
        .fetch_one(pool)
        .await
        .expect("org");

        let password_hash = PasswordHasher.hash_password(password).expect("hash");
        let user_id: Uuid = sqlx::query_scalar(
            "insert into user_account(
               organization_id, email, password_hash, role, is_platform_super_admin
             )
             values ($1, $2, $3, 'admin', $4)
             returning id",
        )
        .bind(org_id)
        .bind(email)
        .bind(password_hash)
        .bind(is_platform_super_admin)
        .fetch_one(pool)
        .await
        .expect("user");

        sqlx::query(
            "insert into org_membership(user_id, organization_id, role)
             values ($1, $2, 'org_admin')
             on conflict (user_id, organization_id) do nothing",
        )
        .bind(user_id)
        .bind(org_id)
        .execute(pool)
        .await
        .expect("membership");

        (user_id, org_id)
    }

    async fn seed_super_admin_no_membership(
        pool: &PgPool,
        email: &str,
        password: &str,
    ) -> (Uuid, Uuid) {
        // Needs a dummy organization_id FK (NOT NULL) but no membership row.
        let slug = format!("sa-park-{}", Uuid::now_v7().simple());
        let org_id: Uuid =
            sqlx::query_scalar("insert into organization(slug, name) values ($1, $2) returning id")
                .bind(&slug)
                .bind("SA parking org")
                .fetch_one(pool)
                .await
                .expect("park org");

        let password_hash = PasswordHasher.hash_password(password).expect("hash");
        let user_id: Uuid = sqlx::query_scalar(
            "insert into user_account(
               organization_id, email, password_hash, role, is_platform_super_admin
             )
             values ($1, $2, $3, 'admin', true)
             returning id",
        )
        .bind(org_id)
        .bind(email)
        .bind(password_hash)
        .fetch_one(pool)
        .await
        .expect("sa user");
        (user_id, org_id)
    }

    async fn create_project_row(pool: &PgPool, org_id: Uuid, slug: &str, name: &str) -> Uuid {
        sqlx::query_scalar(
            "insert into project(
               organization_id, slug, name, description,
               default_embedding_model, include_global_default
             )
             values ($1, $2, $3, $4, 'voyage-4', true)
             returning id",
        )
        .bind(org_id)
        .bind(slug)
        .bind(name)
        .bind(Some(format!("desc-{slug}")))
        .fetch_one(pool)
        .await
        .expect("project")
    }

    async fn create_knowledge_item(
        pool: &PgPool,
        org_id: Uuid,
        project_id: Option<Uuid>,
        title: &str,
        body: &str,
        scope: &str,
        status: &str,
    ) -> Uuid {
        sqlx::query_scalar(
            "insert into knowledge_item(
               organization_id, project_id, scope, status, title, body, category, tags
             )
             values (
               $1, $2, $3::knowledge_scope, $4::knowledge_status, $5, $6, 'general', '{}'::text[]
             )
             returning id",
        )
        .bind(org_id)
        .bind(project_id)
        .bind(scope)
        .bind(status)
        .bind(title)
        .bind(body)
        .fetch_one(pool)
        .await
        .expect("knowledge")
    }

    async fn cleanup_org_cascade(pool: &PgPool, org_id: Uuid) {
        // Order matters for FKs that lack cascade from organization in some tables.
        let _ = sqlx::query("delete from agent_token where organization_id = $1")
            .bind(org_id)
            .execute(pool)
            .await;
        let _ = sqlx::query(
            "delete from approval where knowledge_item_id in (
               select id from knowledge_item where organization_id = $1
             )",
        )
        .bind(org_id)
        .execute(pool)
        .await;
        let _ = sqlx::query(
            "delete from chunk where knowledge_item_id in (
               select id from knowledge_item where organization_id = $1
             )",
        )
        .bind(org_id)
        .execute(pool)
        .await;
        let _ = sqlx::query("delete from knowledge_item where organization_id = $1")
            .bind(org_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("delete from ingestion_job where organization_id = $1")
            .bind(org_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("delete from source_document where organization_id = $1")
            .bind(org_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("delete from project where organization_id = $1")
            .bind(org_id)
            .execute(pool)
            .await;
        let _ = sqlx::query(
            "delete from user_session where user_id in (
               select id from user_account where organization_id = $1
               union
               select user_id from org_membership where organization_id = $1
             )",
        )
        .bind(org_id)
        .execute(pool)
        .await;
        let _ = sqlx::query("delete from org_invite where organization_id = $1")
            .bind(org_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("delete from org_membership where organization_id = $1")
            .bind(org_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("delete from user_account where organization_id = $1")
            .bind(org_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("delete from organization where id = $1")
            .bind(org_id)
            .execute(pool)
            .await;
    }

    async fn login_cookie(app: axum::Router, email: &str, password: &str) -> (String, Value) {
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"email":"{email}","password":"{password}"}}"#
                    )))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(
            response.status(),
            HttpStatus::OK,
            "login failed for {email}"
        );
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .expect("set-cookie")
            .split(';')
            .next()
            .expect("cookie pair")
            .to_owned();
        let body = to_bytes(response.into_body(), 1024 * 64)
            .await
            .expect("body");
        let json: Value = serde_json::from_slice(&body).expect("json");
        (cookie, json)
    }

    async fn json_request(
        app: axum::Router,
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        body: Option<&str>,
    ) -> (HttpStatus, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(cookie) = cookie {
            builder = builder.header(header::COOKIE, cookie);
        }
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let response = app
            .oneshot(
                builder
                    .body(Body::from(body.unwrap_or("").to_owned()))
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 256)
            .await
            .expect("body");
        let json: Value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, json)
    }

    /// VAL-ISOL-001,002,003,015,016,017,029,030,034 + me slug + E2E seed.
    #[tokio::test]
    async fn session_a_cannot_see_org_b_projects_and_super_admin_tenant_denied() {
        let Some(pool) = test_pool().await else {
            eprintln!("skip: QUERIA_DATABASE_URL not set");
            return;
        };

        let password = "correct horse battery staple";
        let tag = Uuid::now_v7().simple().to_string();
        let slug_a = format!("iso-a-{tag}");
        let slug_b = format!("iso-b-{tag}");
        let email_a = format!("admin-a-{tag}@iso.test");
        let email_b = format!("admin-b-{tag}@iso.test");
        let email_sa = format!("sa-{tag}@iso.test");

        let (user_a, org_a) =
            seed_member(&pool, &email_a, password, &slug_a, "Iso Team A", false).await;
        let (user_b, org_b) =
            seed_member(&pool, &email_b, password, &slug_b, "Iso Team B", false).await;
        let (user_sa, park_org) = seed_super_admin_no_membership(&pool, &email_sa, password).await;

        let project_a_slug = format!("proj-a-{tag}");
        let project_b_slug = format!("proj-b-{tag}");
        let project_a_id = create_project_row(&pool, org_a, &project_a_slug, "Project A").await;
        let project_b_id =
            create_project_row(&pool, org_b, &project_b_slug, "Project B Secret Name").await;

        let knowledge_b_title = format!("secret-title-b-{tag}");
        let knowledge_b_body = format!("unique-body-b-token-{tag}");
        let knowledge_b_id = create_knowledge_item(
            &pool,
            org_b,
            Some(project_b_id),
            &knowledge_b_title,
            &knowledge_b_body,
            "project",
            "approved",
        )
        .await;

        let knowledge_a_id = create_knowledge_item(
            &pool,
            org_a,
            Some(project_a_id),
            &format!("a-title-{tag}"),
            &format!("a-body-{tag}"),
            "project",
            "approved",
        )
        .await;

        // Global-scope knowledge under B must not leak as "global" to A.
        // Global items use null project_id (org-global, not instance-global).
        let global_b_title = format!("global-b-{tag}");
        let _global_b = create_knowledge_item(
            &pool,
            org_b,
            None,
            &global_b_title,
            &format!("global-body-b-{tag}"),
            "global",
            "approved",
        )
        .await;

        // Scratch lane under B.
        let scratch_b_title = format!("scratch-b-{tag}");
        let _scratch_b = create_knowledge_item(
            &pool,
            org_b,
            Some(project_b_id),
            &scratch_b_title,
            &format!("scratch-body-b-{tag}"),
            "project",
            "scratch",
        )
        .await;

        let app = build_app_with_pool(AppConfig::default_local(), pool.clone());
        let (cookie_a, login_a) = login_cookie(app.clone(), &email_a, password).await;
        let (cookie_b, login_b) = login_cookie(app.clone(), &email_b, password).await;
        let (cookie_sa, login_sa) = login_cookie(app.clone(), &email_sa, password).await;

        // VAL-ISOL-029 / VAL-ORGS-030: me exposes active org id + slug
        assert_eq!(
            login_a["active_organization_id"].as_str(),
            Some(org_a.to_string().as_str())
        );
        let (status, me_a) =
            json_request(app.clone(), "GET", "/api/v1/auth/me", Some(&cookie_a), None).await;
        assert_eq!(status, HttpStatus::OK);
        assert_eq!(
            me_a["active_organization_id"].as_str(),
            Some(org_a.to_string().as_str())
        );
        assert_eq!(
            me_a["active_organization_slug"].as_str(),
            Some(slug_a.as_str())
        );
        assert!(login_sa["active_organization_id"].is_null());
        assert!(
            login_b["active_organization_id"]
                .as_str()
                .is_some_and(|id| id == org_b.to_string())
        );

        // VAL-ISOL-001 list projects A only
        let (status, list_a) = json_request(
            app.clone(),
            "GET",
            "/api/v1/projects",
            Some(&cookie_a),
            None,
        )
        .await;
        assert_eq!(status, HttpStatus::OK, "list A: {list_a}");
        let a_slugs: Vec<&str> = list_a
            .as_array()
            .expect("array")
            .iter()
            .filter_map(|p| p["slug"].as_str())
            .collect();
        assert!(a_slugs.contains(&project_a_slug.as_str()), "{list_a}");
        assert!(
            !a_slugs.contains(&project_b_slug.as_str()),
            "A must not list B: {list_a}"
        );

        let (status, list_b) = json_request(
            app.clone(),
            "GET",
            "/api/v1/projects",
            Some(&cookie_b),
            None,
        )
        .await;
        assert_eq!(status, HttpStatus::OK);
        let b_slugs: Vec<&str> = list_b
            .as_array()
            .expect("array")
            .iter()
            .filter_map(|p| p["slug"].as_str())
            .collect();
        assert!(b_slugs.contains(&project_b_slug.as_str()));
        assert!(!b_slugs.contains(&project_a_slug.as_str()));

        // VAL-ISOL-002 get foreign project
        let (status, get_b_as_a) = json_request(
            app.clone(),
            "GET",
            &format!("/api/v1/projects/{project_b_slug}"),
            Some(&cookie_a),
            None,
        )
        .await;
        assert!(
            status == HttpStatus::NOT_FOUND || status == HttpStatus::FORBIDDEN,
            "expected deny got {status} {get_b_as_a}"
        );
        let body = get_b_as_a.to_string();
        assert!(
            !body.contains("Project B Secret Name") && !body.contains(&project_b_slug),
            "deny must not leak foreign metadata: {body}"
        );

        // VAL-ISOL-003 create stays home
        let new_slug = format!("created-by-a-{tag}");
        let (status, created) = json_request(
            app.clone(),
            "POST",
            "/api/v1/projects",
            Some(&cookie_a),
            Some(&format!(r#"{{"slug":"{new_slug}","name":"Created By A"}}"#)),
        )
        .await;
        assert_eq!(status, HttpStatus::OK, "create: {created}");
        let (status, list_b2) = json_request(
            app.clone(),
            "GET",
            "/api/v1/projects",
            Some(&cookie_b),
            None,
        )
        .await;
        assert_eq!(status, HttpStatus::OK);
        assert!(
            !list_b2.to_string().contains(&new_slug),
            "B list must not include A create: {list_b2}"
        );

        // VAL-ISOL-005 / 006 knowledge
        let (status, ki_a) = json_request(
            app.clone(),
            "GET",
            "/api/v1/knowledge-items?limit=100",
            Some(&cookie_a),
            None,
        )
        .await;
        assert_eq!(status, HttpStatus::OK, "ki list: {ki_a}");
        let ki_str = ki_a.to_string();
        assert!(!ki_str.contains(&knowledge_b_title));
        assert!(!ki_str.contains(&knowledge_b_body));
        assert!(!ki_str.contains(&global_b_title));
        assert!(!ki_str.contains(&scratch_b_title));
        assert!(
            ki_str.contains(&knowledge_a_id.to_string())
                || ki_str.contains(&format!("a-title-{tag}")),
            "A should see own knowledge: {ki_a}"
        );

        let (status, foreign_ki) = json_request(
            app.clone(),
            "GET",
            &format!("/api/v1/knowledge-items/{knowledge_b_id}"),
            Some(&cookie_a),
            None,
        )
        .await;
        assert!(
            status == HttpStatus::NOT_FOUND || status == HttpStatus::FORBIDDEN,
            "foreign knowledge detail: {status} {foreign_ki}"
        );
        assert!(!foreign_ki.to_string().contains(&knowledge_b_title));

        // VAL-ISOL-009/010 agent tokens: foreign project slug mint fails; lists disjoint
        let (status, mint_foreign) = json_request(
            app.clone(),
            "POST",
            "/api/v1/agent-tokens",
            Some(&cookie_a),
            Some(&format!(
                r#"{{"name":"tok-foreign","project_slugs":["{project_b_slug}"],"expires_in":"7_days"}}"#
            )),
        )
        .await;
        assert!(
            status.is_client_error(),
            "mint foreign slug must fail: {status} {mint_foreign}"
        );

        let (status, mint_a) = json_request(
            app.clone(),
            "POST",
            "/api/v1/agent-tokens",
            Some(&cookie_a),
            Some(&format!(
                r#"{{"name":"tok-a","project_slugs":["{project_a_slug}"],"expires_in":"7_days"}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::OK, "mint A: {mint_a}");
        let token_a_id = mint_a["agent_token"]["id"]
            .as_str()
            .expect("token id")
            .to_owned();
        let raw_token_a = mint_a["token"].as_str().expect("raw token").to_owned();
        assert!(!raw_token_a.is_empty());

        let (status, mint_b) = json_request(
            app.clone(),
            "POST",
            "/api/v1/agent-tokens",
            Some(&cookie_b),
            Some(&format!(
                r#"{{"name":"tok-b","project_slugs":["{project_b_slug}"],"expires_in":"7_days"}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::OK, "mint B: {mint_b}");
        let token_b_id = mint_b["agent_token"]["id"]
            .as_str()
            .expect("token b id")
            .to_owned();
        let raw_token_b = mint_b["token"].as_str().expect("raw token b").to_owned();

        let (status, tokens_a) = json_request(
            app.clone(),
            "GET",
            "/api/v1/agent-tokens",
            Some(&cookie_a),
            None,
        )
        .await;
        assert_eq!(status, HttpStatus::OK);
        let tokens_a_str = tokens_a.to_string();
        assert!(tokens_a_str.contains(&token_a_id));
        assert!(!tokens_a_str.contains(&token_b_id));

        let (status, _get_b_tok) = json_request(
            app.clone(),
            "GET",
            &format!("/api/v1/agent-tokens/{token_b_id}"),
            Some(&cookie_a),
            None,
        )
        .await;
        assert!(
            status == HttpStatus::NOT_FOUND || status == HttpStatus::FORBIDDEN,
            "A get B token: {status}"
        );

        let (status, _del_b_tok) = json_request(
            app.clone(),
            "DELETE",
            &format!("/api/v1/agent-tokens/{token_b_id}"),
            Some(&cookie_a),
            None,
        )
        .await;
        assert!(
            status == HttpStatus::NOT_FOUND || status == HttpStatus::FORBIDDEN,
            "A delete B token: {status}"
        );
        // B token still listable by B
        let (status, tokens_b) = json_request(
            app.clone(),
            "GET",
            "/api/v1/agent-tokens",
            Some(&cookie_b),
            None,
        )
        .await;
        assert_eq!(status, HttpStatus::OK);
        assert!(tokens_b.to_string().contains(&token_b_id));

        // VAL-ISOL-015/016/017 super-admin without membership
        let (status, sa_projects) = json_request(
            app.clone(),
            "GET",
            "/api/v1/projects",
            Some(&cookie_sa),
            None,
        )
        .await;
        assert_eq!(
            status,
            HttpStatus::FORBIDDEN,
            "SA tenant projects must 403: {sa_projects}"
        );
        let (status, _sa_ki) = json_request(
            app.clone(),
            "GET",
            "/api/v1/knowledge-items",
            Some(&cookie_sa),
            None,
        )
        .await;
        assert_eq!(status, HttpStatus::FORBIDDEN);
        let (status, sa_dash) = json_request(
            app.clone(),
            "GET",
            "/api/v1/dashboard/summary",
            Some(&cookie_sa),
            None,
        )
        .await;
        assert_eq!(status, HttpStatus::FORBIDDEN, "SA dashboard: {sa_dash}");
        let (status, _sa_tokens) = json_request(
            app.clone(),
            "GET",
            "/api/v1/agent-tokens",
            Some(&cookie_sa),
            None,
        )
        .await;
        assert_eq!(status, HttpStatus::FORBIDDEN);
        let (status, sa_retrieve) = json_request(
            app.clone(),
            "POST",
            "/api/v1/retrieve-context",
            Some(&cookie_sa),
            Some(&format!(
                r#"{{"project_id":"{project_b_id}","query":"{knowledge_b_body}","include_global":true,"include_scratch":false,"limit":5}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::FORBIDDEN, "SA retrieve: {sa_retrieve}");

        // Orgs APIs still work for SA
        let (status, sa_orgs) =
            json_request(app.clone(), "GET", "/api/v1/orgs", Some(&cookie_sa), None).await;
        assert_eq!(status, HttpStatus::OK, "SA list orgs: {sa_orgs}");
        assert!(sa_orgs.to_string().contains(&slug_a) || sa_orgs.as_array().is_some());

        // VAL-ISOL-018 non-SA cannot list orgs
        let (status, _a_orgs) =
            json_request(app.clone(), "GET", "/api/v1/orgs", Some(&cookie_a), None).await;
        assert_eq!(status, HttpStatus::FORBIDDEN);

        // VAL-ISOL-022 dashboard A unchanged by B seed size relative to foreign
        let (status, dash_a) = json_request(
            app.clone(),
            "GET",
            "/api/v1/dashboard/summary",
            Some(&cookie_a),
            None,
        )
        .await;
        assert_eq!(status, HttpStatus::OK, "dash A: {dash_a}");
        // A has at least its projects (seeded + created)
        assert!(
            dash_a["project_count"].as_i64().unwrap_or(0) >= 1,
            "{dash_a}"
        );

        // VAL-ISOL-034 agent-setup public
        let (status, setup) =
            json_request(app.clone(), "GET", "/api/v1/docs/agent-setup", None, None).await;
        assert_eq!(status, HttpStatus::OK, "agent-setup public: {setup}");

        // VAL-ISOL-007 / 008 retrieval: A probe on B slug denied
        let (status, probe) = json_request(
            app.clone(),
            "POST",
            &format!("/api/v1/projects/{project_b_slug}/retrieval/probe"),
            Some(&cookie_a),
            Some(&format!(r#"{{"query":"{knowledge_b_body}","limit":5}}"#)),
        )
        .await;
        assert!(
            status == HttpStatus::NOT_FOUND || status == HttpStatus::FORBIDDEN,
            "A probe B: {status} {probe}"
        );

        // Source register under A then B cannot list by foreign slug
        let source_uri = format!("file:///tmp/iso-{tag}.md");
        let (status, src) = json_request(
            app.clone(),
            "POST",
            "/api/v1/sources",
            Some(&cookie_a),
            Some(&format!(
                r#"{{"project_slug":"{project_a_slug}","kind":"manual_note","uri":"{source_uri}","title":"A note","content_hash":"hash-{tag}"}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::OK, "register source: {src}");
        let source_a_id = src["id"].as_str().unwrap_or_default().to_owned();

        let (status, src_list_b) = json_request(
            app.clone(),
            "GET",
            &format!("/api/v1/sources?project_slug={project_a_slug}"),
            Some(&cookie_b),
            None,
        )
        .await;
        // B querying A's project_slug either empty or 400/403/404 — never contain source id
        if status == HttpStatus::OK {
            assert!(
                !src_list_b.to_string().contains(&source_a_id),
                "B must not list A sources: {src_list_b}"
            );
        } else {
            assert!(status.is_client_error(), "unexpected {status}");
        }
        if !source_a_id.is_empty() {
            let (status, _src_b_get) = json_request(
                app.clone(),
                "GET",
                &format!("/api/v1/sources/{source_a_id}"),
                Some(&cookie_b),
                None,
            )
            .await;
            assert!(
                status == HttpStatus::NOT_FOUND || status == HttpStatus::FORBIDDEN,
                "B get A source: {status}"
            );
        }

        // Approvals/jobs/audit lists empty of foreign — just ensure 200 and no panic
        for path in [
            "/api/v1/approvals",
            "/api/v1/ingestion-jobs",
            "/api/v1/audit-logs",
        ] {
            let (status, body) =
                json_request(app.clone(), "GET", path, Some(&cookie_a), None).await;
            assert_eq!(status, HttpStatus::OK, "{path}: {body}");
        }

        // Silence unused
        let _ = (
            user_a,
            user_b,
            user_sa,
            raw_token_a,
            raw_token_b,
            project_a_id,
        );

        cleanup_org_cascade(&pool, org_a).await;
        cleanup_org_cascade(&pool, org_b).await;
        cleanup_org_cascade(&pool, park_org).await;
    }

    /// VAL-CROSS-001/005 / VAL-ISOL-019/020: create Team B via API, accept, operate, second-org reject.
    #[tokio::test]
    async fn e2e_create_team_b_accept_operate_and_second_org_conflict() {
        let Some(pool) = test_pool().await else {
            eprintln!("skip: QUERIA_DATABASE_URL not set");
            return;
        };
        let password = "correct horse battery staple";
        let tag = Uuid::now_v7().simple().to_string();
        let sa_email = format!("e2e-sa-{tag}@iso.test");
        let team_a_slug = format!("e2e-a-{tag}");
        let team_b_slug = format!("e2e-b-{tag}");
        let admin_a_email = format!("e2e-admin-a-{tag}@iso.test");
        let admin_b_email = format!("e2e-admin-b-{tag}@iso.test");

        let (sa_user, park) = seed_super_admin_no_membership(&pool, &sa_email, password).await;
        // Pre-seed team A member for second-org conflict
        let (user_a, org_a) = seed_member(
            &pool,
            &admin_a_email,
            password,
            &team_a_slug,
            "E2E A",
            false,
        )
        .await;
        let proj_a = format!("e2e-proj-a-{tag}");
        let _ = create_project_row(&pool, org_a, &proj_a, "E2E Project A").await;

        let app = build_app_with_pool(AppConfig::default_local(), pool.clone());
        let (cookie_sa, _) = login_cookie(app.clone(), &sa_email, password).await;

        // Create org B via platform API
        let (status, create_b) = json_request(
            app.clone(),
            "POST",
            "/api/v1/orgs",
            Some(&cookie_sa),
            Some(&format!(
                r#"{{"slug":"{team_b_slug}","name":"E2E Team B","first_admin_email":"{admin_b_email}"}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::OK, "create org B: {create_b}");
        let invite_token = create_b["invite_token"]
            .as_str()
            .expect("invite token once")
            .to_owned();
        let org_b_id = create_b["organization"]["id"]
            .as_str()
            .expect("org id")
            .to_owned();

        // Accept as new Team B admin
        let (status, accepted) = json_request(
            app.clone(),
            "POST",
            "/api/v1/invites/accept",
            None,
            Some(&format!(
                r#"{{"token":"{invite_token}","password":"{password}","name":"Admin B"}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::OK, "accept: {accepted}");
        assert_eq!(accepted["accepted"], true);

        let (cookie_b, me_b) = login_cookie(app.clone(), &admin_b_email, password).await;
        assert_eq!(
            me_b["active_organization_id"].as_str(),
            Some(org_b_id.as_str())
        );
        assert_eq!(
            me_b["active_organization_slug"].as_str(),
            Some(team_b_slug.as_str())
        );

        // Operate: create project under B only
        let proj_b = format!("e2e-proj-b-{tag}");
        let (status, created_proj) = json_request(
            app.clone(),
            "POST",
            "/api/v1/projects",
            Some(&cookie_b),
            Some(&format!(r#"{{"slug":"{proj_b}","name":"E2E Project B"}}"#)),
        )
        .await;
        assert_eq!(status, HttpStatus::OK, "create proj B: {created_proj}");

        let (status, list_b) = json_request(
            app.clone(),
            "GET",
            "/api/v1/projects",
            Some(&cookie_b),
            None,
        )
        .await;
        assert_eq!(status, HttpStatus::OK);
        assert!(list_b.to_string().contains(&proj_b));
        assert!(!list_b.to_string().contains(&proj_a));

        // A still only A
        let (cookie_a, _) = login_cookie(app.clone(), &admin_a_email, password).await;
        let (status, list_a) = json_request(
            app.clone(),
            "GET",
            "/api/v1/projects",
            Some(&cookie_a),
            None,
        )
        .await;
        assert_eq!(status, HttpStatus::OK);
        assert!(list_a.to_string().contains(&proj_a));
        assert!(!list_a.to_string().contains(&proj_b));

        // Second-org invite: try invite A admin email into B — accept must fail closed
        let (status, inv2) = json_request(
            app.clone(),
            "POST",
            &format!("/api/v1/orgs/{team_b_slug}/invites"),
            Some(&cookie_sa),
            Some(&format!(
                r#"{{"email":"{admin_a_email}","role":"org_member"}}"#
            )),
        )
        .await;
        assert_eq!(status, HttpStatus::OK, "second invite: {inv2}");
        let invite2 = inv2["invite_token"].as_str().expect("token2").to_owned();
        let (status, conflict) = json_request(
            app.clone(),
            "POST",
            "/api/v1/invites/accept",
            None,
            Some(&format!(
                r#"{{"token":"{invite2}","password":"{password}"}}"#
            )),
        )
        .await;
        assert!(
            status.is_client_error(),
            "second-org accept must fail: {status} {conflict}"
        );
        // A still home A
        let (cookie_a2, me_a2) = login_cookie(app.clone(), &admin_a_email, password).await;
        assert_eq!(
            me_a2["active_organization_id"].as_str(),
            Some(org_a.to_string().as_str())
        );
        let (status, list_a2) =
            json_request(app, "GET", "/api/v1/projects", Some(&cookie_a2), None).await;
        assert_eq!(status, HttpStatus::OK);
        assert!(!list_a2.to_string().contains(&proj_b));

        let _ = (sa_user, user_a);
        // cleanup B by slug lookup
        let org_b_uuid = Uuid::parse_str(&org_b_id).expect("uuid");
        cleanup_org_cascade(&pool, org_b_uuid).await;
        cleanup_org_cascade(&pool, org_a).await;
        cleanup_org_cascade(&pool, park).await;
    }
}
