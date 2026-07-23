use queria_core::ids::{ApprovalId, KnowledgeItemId};
use queria_core::{QueriaError, QueriaResult};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use super::super::types::{
    ApprovalRecord, ApprovedKnowledgeRecord, KnowledgeItemRecord, approval_for_update,
    approval_from_row, ensure_approval_source_document, insert_approval_audit_log,
    knowledge_item_from_row, to_infrastructure_error,
};
use super::PgProjectRepository;

impl PgProjectRepository {
    pub async fn list_approvals(
        &self,
        user_id: Uuid,
        status: Option<&str>,
    ) -> QueriaResult<Vec<ApprovalRecord>> {
        sqlx::query(
            "select a.id, a.knowledge_item_id, ki.project_id, ki.source_document_id,
                    ki.scope::text as scope, ki.status::text as knowledge_status,
                    ki.title, ki.body, ki.category, ki.tags,
                    a.requested_by, a.reviewer_user_id, a.status::text as approval_status,
                    a.reason, a.created_at, a.decided_at, ki.approved_at
             from approval a
             join knowledge_item ki on ki.id = a.knowledge_item_id
             join org_membership m on m.organization_id = ki.organization_id
             where m.user_id = $1
               and ($2::text is null or a.status::text = $2)
             order by a.created_at desc",
        )
        .bind(user_id)
        .bind(status)
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .into_iter()
        .map(approval_from_row)
        .collect()
    }

    pub async fn get_approval(
        &self,
        user_id: Uuid,
        approval_id: ApprovalId,
    ) -> QueriaResult<Option<ApprovalRecord>> {
        sqlx::query(
            "select a.id, a.knowledge_item_id, ki.project_id, ki.source_document_id,
                    ki.scope::text as scope, ki.status::text as knowledge_status,
                    ki.title, ki.body, ki.category, ki.tags,
                    a.requested_by, a.reviewer_user_id, a.status::text as approval_status,
                    a.reason, a.created_at, a.decided_at, ki.approved_at
             from approval a
             join knowledge_item ki on ki.id = a.knowledge_item_id
             join org_membership m on m.organization_id = ki.organization_id
             where m.user_id = $1
               and a.id = $2",
        )
        .bind(user_id)
        .bind(approval_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(approval_from_row)
        .transpose()
    }

    pub async fn get_knowledge_item(
        &self,
        user_id: Uuid,
        knowledge_item_id: KnowledgeItemId,
    ) -> QueriaResult<Option<KnowledgeItemRecord>> {
        sqlx::query(
            "select ki.id, ki.project_id, ki.source_document_id, ki.scope::text as scope,
                    ki.status::text as status, ki.title, ki.body, ki.category,
                    ki.tags, ki.approved_at, ki.created_at, ki.updated_at
             from knowledge_item ki
             join org_membership m on m.organization_id = ki.organization_id
             where m.user_id = $1
               and ki.id = $2",
        )
        .bind(user_id)
        .bind(knowledge_item_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(knowledge_item_from_row)
        .transpose()
    }

    pub async fn approve_approval(
        &self,
        user_id: Uuid,
        approval_id: ApprovalId,
    ) -> QueriaResult<Option<ApprovedKnowledgeRecord>> {
        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;
        let approval = approval_for_update(&mut transaction, user_id, approval_id).await?;
        let Some(mut approval) = approval else {
            return Ok(None);
        };

        if approval.approval_status != "pending" || approval.knowledge_status != "proposed" {
            return Err(QueriaError::Validation(
                "approval is not pending for a proposed knowledge item".to_owned(),
            ));
        }

        let source_document_id =
            ensure_approval_source_document(&mut transaction, user_id, &approval).await?;

        sqlx::query(
            "update knowledge_item
             set status = 'approved',
                 source_document_id = $2,
                 approved_at = now(),
                 updated_at = now()
             where id = $1",
        )
        .bind(approval.knowledge_item_id)
        .bind(source_document_id)
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        sqlx::query(
            "update approval
             set status = 'approved',
                 reviewer_user_id = $2,
                 decided_at = now()
             where id = $1",
        )
        .bind(approval.id)
        .bind(user_id)
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let chunk_id = sqlx::query(
            "insert into chunk(
               knowledge_item_id, source_document_id, chunk_index, body,
               token_count, content_hash, metadata
             )
             values ($1, $2, 0, $3, 0, $4, $5)
             on conflict (knowledge_item_id, chunk_index) do update
             set source_document_id = excluded.source_document_id,
                 body = excluded.body,
                 content_hash = excluded.content_hash,
                 metadata = excluded.metadata
             returning id",
        )
        .bind(approval.knowledge_item_id)
        .bind(source_document_id)
        .bind(&approval.body)
        .bind(format!(
            "knowledge_item:{}:approved:v1",
            approval.knowledge_item_id
        ))
        .bind(json!({
            "approval_id": approval.id,
            "line_start": 1,
            "line_end": approval.body.lines().count().max(1)
        }))
        .fetch_one(&mut *transaction)
        .await
        .and_then(|row| row.try_get::<Uuid, _>("id"))
        .map_err(to_infrastructure_error)?;

        insert_approval_audit_log(
            &mut transaction,
            user_id,
            "approval.approved",
            approval.id,
            approval.knowledge_item_id,
            json!({
                "chunk_id": chunk_id,
                "source_document_id": source_document_id
            }),
        )
        .await?;

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;

        approval.source_document_id = Some(source_document_id);
        approval.approval_status = "approved".to_owned();
        approval.knowledge_status = "approved".to_owned();
        approval.reviewer_user_id = Some(user_id);

        Ok(Some(ApprovedKnowledgeRecord {
            approval,
            chunk_id,
            source_document_id,
        }))
    }

    pub async fn reject_approval(
        &self,
        user_id: Uuid,
        approval_id: ApprovalId,
        reason: String,
    ) -> QueriaResult<Option<ApprovalRecord>> {
        let reason = reason.trim().to_owned();
        if reason.is_empty() || reason.len() > 2_000 {
            return Err(QueriaError::Validation(
                "rejection reason must be between 1 and 2000 bytes".to_owned(),
            ));
        }

        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;
        let approval = approval_for_update(&mut transaction, user_id, approval_id).await?;
        let Some(mut approval) = approval else {
            return Ok(None);
        };

        if approval.approval_status != "pending" || approval.knowledge_status != "proposed" {
            return Err(QueriaError::Validation(
                "approval is not pending for a proposed knowledge item".to_owned(),
            ));
        }

        sqlx::query(
            "update knowledge_item
             set status = 'rejected',
                 updated_at = now()
             where id = $1",
        )
        .bind(approval.knowledge_item_id)
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        sqlx::query(
            "update approval
             set status = 'rejected',
                 reviewer_user_id = $2,
                 reason = $3,
                 decided_at = now()
             where id = $1",
        )
        .bind(approval.id)
        .bind(user_id)
        .bind(&reason)
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        insert_approval_audit_log(
            &mut transaction,
            user_id,
            "approval.rejected",
            approval.id,
            approval.knowledge_item_id,
            json!({ "reason": reason }),
        )
        .await?;

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;

        approval.approval_status = "rejected".to_owned();
        approval.knowledge_status = "rejected".to_owned();
        approval.reviewer_user_id = Some(user_id);
        approval.reason = Some(reason);

        Ok(Some(approval))
    }
}
