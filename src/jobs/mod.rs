pub mod email;
pub mod invoice_email;
pub mod overdue;
pub mod recurring;
pub mod webhook_delivery;

use std::str::FromStr;
use std::sync::Arc;

use apalis::layers::retry::RetryPolicy;
use apalis::layers::WorkerBuilderExt;
use apalis::prelude::{Storage, WorkerBuilder, WorkerFactoryFn};
use apalis_cron::{CronStream, Schedule};
use apalis_redis::RedisStorage;
use reqwest::Client;
use uuid::Uuid;

use crate::db::DbPool;
use crate::jobs::email::{EmailSender, LogEmailSender};
use crate::jobs::invoice_email::SendInvoiceEmail;
use crate::jobs::recurring::CreateRecurring;
use crate::jobs::webhook_delivery::DeliverWebhook;

/// Handle cloned into app state so HTTP handlers can enqueue jobs.
#[derive(Clone)]
pub struct JobQueues {
    pub invoice_email: RedisStorage<SendInvoiceEmail>,
    pub webhook_delivery: RedisStorage<DeliverWebhook>,
    pub recurring_create: RedisStorage<CreateRecurring>,
}

pub async fn connect(redis_url: &str) -> anyhow::Result<JobQueues> {
    let conn = apalis_redis::connect(redis_url).await?;
    Ok(JobQueues {
        invoice_email: RedisStorage::new(conn.clone()),
        webhook_delivery: RedisStorage::new(conn.clone()),
        recurring_create: RedisStorage::new(conn),
    })
}

/// Spawns every background worker on the current tokio runtime and returns
/// immediately. Each worker runs for the lifetime of the process.
pub fn spawn_workers(queues: JobQueues, pool: DbPool, app_url: String) {
    let sender: Arc<dyn EmailSender> = Arc::new(LogEmailSender);

    // --- invoice email worker ---
    let invoice_email_pool = pool.clone();
    let invoice_email_sender = sender.clone();
    let invoice_email_app_url = app_url.clone();
    let invoice_email_storage = queues.invoice_email.clone();
    tokio::spawn(async move {
        WorkerBuilder::new("invoice-email")
            .concurrency(5)
            .retry(RetryPolicy::retries(3))
            .data(invoice_email_pool)
            .data(invoice_email_sender)
            .data(invoice_email_app_url)
            .backend(invoice_email_storage)
            .build_fn(invoice_email::handle)
            .run()
            .await;
        tracing::warn!("invoice-email worker terminated");
    });

    // --- webhook delivery worker ---
    // Uses a shared reqwest client for connection pooling across deliveries.
    let http_client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("build reqwest client");
    let webhook_storage = queues.webhook_delivery.clone();
    tokio::spawn(async move {
        WorkerBuilder::new("webhook-delivery")
            .concurrency(10)
            .retry(RetryPolicy::retries(8))
            .data(http_client)
            .backend(webhook_storage)
            .build_fn(webhook_delivery::handle)
            .run()
            .await;
        tracing::warn!("webhook-delivery worker terminated");
    });

    // --- overdue cron (hourly) ---
    let overdue_pool = pool.clone();
    let overdue_queues = queues.clone();
    tokio::spawn(async move {
        let schedule = Schedule::from_str("0 0 * * * *").expect("valid cron");
        WorkerBuilder::new("overdue-scan")
            .data(overdue_pool)
            .data(overdue_queues)
            .backend(CronStream::new(schedule))
            .build_fn(overdue::handle)
            .run()
            .await;
        tracing::warn!("overdue-scan worker terminated");
    });

    // --- recurring-invoice materializer (queue worker) ---
    let recurring_create_pool = pool.clone();
    let recurring_create_queues = queues.clone();
    let recurring_create_storage = queues.recurring_create.clone();
    tokio::spawn(async move {
        WorkerBuilder::new("recurring-create")
            .concurrency(3)
            .retry(RetryPolicy::retries(3))
            .data(recurring_create_pool)
            .data(recurring_create_queues)
            .backend(recurring_create_storage)
            .build_fn(recurring::create)
            .run()
            .await;
        tracing::warn!("recurring-create worker terminated");
    });

    // --- recurring-invoice scanner (cron every 15 min) ---
    let recurring_scan_pool = pool.clone();
    let recurring_scan_queues = queues.clone();
    tokio::spawn(async move {
        let schedule = Schedule::from_str("0 */15 * * * *").expect("valid cron");
        WorkerBuilder::new("recurring-scan")
            .data(recurring_scan_pool)
            .data(recurring_scan_queues)
            .backend(CronStream::new(schedule))
            .build_fn(recurring::scan)
            .run()
            .await;
        tracing::warn!("recurring-scan worker terminated");
    });
}

/// Enqueue a send-invoice-email job.
pub async fn enqueue_invoice_email(
    queues: &JobQueues,
    invoice_id: Uuid,
) -> anyhow::Result<()> {
    let mut storage = queues.invoice_email.clone();
    storage.push(SendInvoiceEmail { invoice_id }).await?;
    Ok(())
}

/// Fan an event out to every matching active webhook subscription.
///
/// Queries `webhook_subscriptions` for active subs in this org that subscribed
/// to `event`, then enqueues one delivery job per sub. Snapshot of url/secret
/// goes into the payload so subsequent subscription edits don't retarget
/// in-flight retries.
///
/// Errors are logged but not propagated — dispatching webhooks is a side
/// effect of the business operation, never its success condition.
pub async fn dispatch_webhooks(
    queues: &JobQueues,
    pool: &DbPool,
    org_id: Uuid,
    event: &str,
    data: serde_json::Value,
) {
    let subs: Vec<(Uuid, String, String)> = match sqlx::query_as(
        r#"
        SELECT id, url, secret
        FROM webhook_subscriptions
        WHERE org_id = $1 AND status = 'active' AND $2 = ANY(events)
        "#,
    )
    .bind(org_id)
    .bind(event)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, event, %org_id, "failed to query webhook subscriptions");
            return;
        }
    };

    if subs.is_empty() {
        return;
    }

    let payload = serde_json::json!({
        "event": event,
        "data": data,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    })
    .to_string();

    let mut storage = queues.webhook_delivery.clone();
    for (sub_id, url, secret) in subs {
        let job = DeliverWebhook {
            subscription_id: sub_id,
            org_id,
            event: event.to_string(),
            url,
            secret,
            payload: payload.clone(),
            delivery_id: Uuid::new_v4(),
        };
        if let Err(e) = storage.push(job).await {
            tracing::error!(error = %e, event, %sub_id, "failed to enqueue webhook delivery");
        }
    }
}
