use std::sync::Arc;

use apalis::prelude::{BoxDynError, Data, Error};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::db::DbPool;
use crate::jobs::email::{Email, EmailSender};

/// Queue payload: "send the invoice email for this invoice."
/// We keep this small — the job re-fetches the invoice + client from DB so
/// the payload stays immutable even if the invoice gets edited before the
/// job runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendInvoiceEmail {
    pub invoice_id: Uuid,
}

#[derive(FromRow)]
struct InvoiceEmailData {
    invoice_number: String,
    total: rust_decimal::Decimal,
    currency: String,
    due_date: chrono::DateTime<chrono::Utc>,
    client_email: String,
    client_name: String,
}

pub async fn handle(
    job: SendInvoiceEmail,
    pool: Data<DbPool>,
    sender: Data<Arc<dyn EmailSender>>,
    app_url: Data<String>,
) -> Result<(), Error> {
    let row: Option<InvoiceEmailData> = sqlx::query_as(
        r#"
        SELECT i.invoice_number, i.total, i.currency, i.due_date,
               c.email AS client_email, c.name AS client_name
        FROM invoices i
        JOIN clients c ON c.id = i.client_id
        WHERE i.id = $1
        "#,
    )
    .bind(job.invoice_id)
    .fetch_optional(&*pool)
    .await
    .map_err(|e| Error::from(Box::new(e) as BoxDynError))?;

    let Some(data) = row else {
        tracing::warn!(invoice_id = %job.invoice_id, "invoice not found for email job");
        return Ok(());
    };

    let subject = format!("Invoice {} from your vendor", data.invoice_number);
    let body = format!(
        "Hi {name},\n\n\
         You have a new invoice:\n\
           Number: {num}\n\
           Amount: {amount} {currency}\n\
           Due: {due}\n\n\
         Pay online: {app_url}/invoices/{id}/pay\n\n\
         Thanks.\n",
        name = data.client_name,
        num = data.invoice_number,
        amount = data.total,
        currency = data.currency,
        due = data.due_date.format("%Y-%m-%d"),
        app_url = &**app_url,
        id = job.invoice_id,
    );

    sender
        .send(Email {
            to: data.client_email,
            subject,
            body,
        })
        .await
        .map_err(|e| Error::from(Box::new(std::io::Error::other(e.to_string())) as BoxDynError))?;

    Ok(())
}
