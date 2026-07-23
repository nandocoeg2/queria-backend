//! PgProjectRepository: project-scoped DB operations (split by domain).
//!
//! Callers keep using `queria_db::repositories::PgProjectRepository` via the façade re-export.
//! Submodules are mechanical `impl` splits only: same type, no new trait/interface layer.

mod agent_access;
mod agent_index;
mod approvals;
mod crud;
mod needs_review;
mod scratch;
mod search;
mod seed;
mod sources;
#[cfg(test)]
mod tests;
mod tokens;

use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct PgProjectRepository {
    pool: PgPool,
}

impl PgProjectRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}
