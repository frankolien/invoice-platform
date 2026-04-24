use apalis::prelude::{BoxDynError, Data, Error};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::db::DbPool;
use crate::jobs::{JobQueues, dispatch_webhooks};

/// apalis-cron's CronStream emits `DateTime<Utc>` ticks; we wrap it in a
/// newtype that implements `From<DateTime<Utc>>` so cron can construct it.
#[derive(Default, Debug, Clone)]
pub struct OverdueTick(pub DateTime<Utc>);

impl From<DateTime<Utc>> for OverdueTick {
    fn from(t: DateTime<Utc>) -> Self {
        OverdueTick(t)
    }
}

pub async fn handle(
    _tick: OverdueTick,
    pool: Data<DbPool>,
    queues: Data<JobQueues>,
) -> Result<(), Error> {
    // RETURNING lets us fire invoice.overdue exactly once per transition.
    // Only rows actually flipped are returned, so retrying the cron on
    // restart won't double-fire.
    let transitioned: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        r#"
        UPDATE invoices
        SET status = 'overdue', updated_at = now()
        WHERE status IN ('sent', 'viewed', 'partially_paid')
          AND due_date < now()
        RETURNING id, org_id, invoice_number
        "#,
    )
    .fetch_all(&*pool)
    .await
    .map_err(|e| Error::from(Box::new(e) as BoxDynError))?;

    if transitioned.is_empty() {
        return Ok(());
    }

    tracing::info!(
        count = transitioned.len(),
        "transitioned invoices to overdue"
    );

    for (invoice_id, org_id, invoice_number) in transitioned {
        dispatch_webhooks(
            &queues,
            &pool,
            org_id,
            "invoice.overdue",
            serde_json::json!({
                "invoice_id": invoice_id,
                "invoice_number": invoice_number,
            }),
        )
        .await;
    }

    Ok(())
}
