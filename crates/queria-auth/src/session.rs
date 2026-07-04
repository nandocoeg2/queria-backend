use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionClaims {
    pub user_id: String,
    pub email: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}
