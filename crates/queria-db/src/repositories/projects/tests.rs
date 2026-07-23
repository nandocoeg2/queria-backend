#[cfg(test)]
mod index_memory_tests {
    use super::super::PgProjectRepository;

    /// VAL-DL-018 / IMP-22: lookup is keyed by project + content_hash + scratch only.
    #[test]
    fn idempotent_lookup_filters_scratch_and_hash() {
        let sql = PgProjectRepository::index_memory_idempotent_lookup_sql();
        assert!(sql.contains("ki.status = 'scratch'"));
        assert!(sql.contains("ki.content_hash = $2"));
        assert!(sql.contains("ki.project_id = $1"));
        assert!(!sql.contains("status = 'approved'"));
    }

    /// VAL-DL-008 / VAL-DL-013: insert always project-scoped scratch.
    #[test]
    fn insert_sql_is_project_scoped_scratch() {
        let sql = PgProjectRepository::index_memory_insert_sql_snippet();
        assert!(sql.contains("'project'"));
        assert!(sql.contains("'scratch'"));
        assert!(!sql.contains("'global'"));
        assert!(!sql.contains("'approved'"));
    }
}

#[cfg(test)]
mod needs_review_tests {
    use super::super::PgProjectRepository;

    /// IMP-L4: list filters needs_review only.
    #[test]
    fn list_needs_review_filters_status() {
        assert_eq!(
            PgProjectRepository::list_needs_review_sql_contract(),
            "and ki.status = 'needs_review'"
        );
    }

    /// IMP-L4: promote sets approved (trusted path).
    #[test]
    fn promote_sets_approved() {
        assert!(
            PgProjectRepository::promote_needs_review_status_sql().contains("status = 'approved'")
        );
    }

    /// IMP-L4: reject sets rejected.
    #[test]
    fn reject_sets_rejected() {
        assert!(
            PgProjectRepository::reject_needs_review_status_sql().contains("status = 'rejected'")
        );
    }

    /// Re-index with new hash supersedes prior needs_review for same logical_path.
    #[test]
    fn supersede_prior_needs_review_sql_contract() {
        let sql = PgProjectRepository::supersede_prior_needs_review_sql();
        assert!(sql.contains("status = 'superseded'"));
        assert!(sql.contains("ki.status = 'needs_review'"));
        assert!(sql.contains("ki.project_id = $1"));
        assert!(sql.contains("c.metadata->>'logical_path' = $2"));
        assert!(sql.contains("c.content_hash is distinct from $3"));
        // Must not touch approved / promoted items.
        assert!(!sql.contains("status = 'approved'"));
        assert!(!sql.contains("status != 'needs_review'"));
    }

    /// Bulk without origin+commit rejects unless force_project_all.
    #[test]
    fn bulk_rejects_without_origin_and_commit() {
        let err = PgProjectRepository::bulk_origin_commit_allowed(None, None, false)
            .expect_err("empty origin+commit without force");
        assert_eq!(err, "origin_url or commit_sha required for bulk");

        assert!(
            PgProjectRepository::bulk_origin_commit_allowed(None, None, true).is_ok(),
            "force_project_all allows empty origin+commit"
        );
        assert!(
            PgProjectRepository::bulk_origin_commit_allowed(Some("git@h:a.git"), None, false)
                .is_ok()
        );
        assert!(
            PgProjectRepository::bulk_origin_commit_allowed(None, Some("abc123"), false).is_ok()
        );
        assert!(
            PgProjectRepository::bulk_origin_commit_allowed(Some("  "), Some(""), false).is_err(),
            "whitespace-only origin/commit still empty"
        );
    }
}
