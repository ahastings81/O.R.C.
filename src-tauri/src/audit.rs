use chrono::Utc;
use uuid::Uuid;

use crate::models::AuditEvent;

pub fn audit_event(
    category: impl Into<String>,
    source: impl Into<String>,
    outcome: Option<String>,
    message: impl Into<String>,
    request_id: Option<String>,
    worker_id: Option<String>,
) -> AuditEvent {
    AuditEvent {
        id: Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        category: category.into(),
        source: source.into(),
        outcome,
        message: message.into(),
        request_id,
        worker_id,
    }
}
