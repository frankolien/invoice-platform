use std::time::Duration;

use apalis::prelude::{BoxDynError, Data, Error};
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);

/// One outbound webhook delivery attempt.
///
/// We re-snapshot `url` + `secret` into the job payload so a subscription edit
/// after the job was enqueued doesn't mis-route the delivery to a new URL or
/// break signing with a new secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliverWebhook {
    pub subscription_id: Uuid,
    pub org_id: Uuid,
    pub event: String,
    pub url: String,
    pub secret: String,
    pub payload: String,
    pub delivery_id: Uuid,
}

pub async fn handle(
    job: DeliverWebhook,
    client: Data<Client>,
) -> Result<(), Error> {
    let status = deliver_with_client(&client, &job.url, &job.secret, &job.event, &job.payload)
        .await
        .map_err(|e| Error::from(Box::new(std::io::Error::other(e.to_string())) as BoxDynError))?;

    if !(200..300).contains(&status) {
        // Non-2xx -> return Err so apalis retries with backoff.
        return Err(Error::from(Box::new(std::io::Error::other(format!(
            "webhook delivery non-2xx: {status}"
        ))) as BoxDynError));
    }

    tracing::info!(
        delivery_id = %job.delivery_id,
        event = %job.event,
        url = %job.url,
        status,
        "webhook delivered"
    );
    Ok(())
}

/// Synchronous single-shot delivery used by the /test endpoint. Same signing
/// + headers as the queue path, but errors bubble to the HTTP response
/// instead of retrying.
pub async fn deliver_once(
    url: &str,
    secret: &str,
    event: &str,
    payload: &str,
) -> anyhow::Result<u16> {
    let client = Client::builder().timeout(DELIVERY_TIMEOUT).build()?;
    deliver_with_client(&client, url, secret, event, payload).await
}

async fn deliver_with_client(
    client: &Client,
    url: &str,
    secret: &str,
    event: &str,
    payload: &str,
) -> anyhow::Result<u16> {
    let timestamp = chrono::Utc::now().timestamp().to_string();
    let signature = sign(&timestamp, payload, secret)?;
    let delivery_id = Uuid::new_v4();

    let resp = client
        .post(url)
        .header("content-type", "application/json")
        .header("x-signature", signature)
        .header("x-timestamp", &timestamp)
        .header("x-event-type", event)
        .header("x-delivery-id", delivery_id.to_string())
        .body(payload.to_string())
        .timeout(DELIVERY_TIMEOUT)
        .send()
        .await?;

    Ok(resp.status().as_u16())
}

fn sign(timestamp: &str, payload: &str, secret: &str) -> anyhow::Result<String> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())?;
    mac.update(format!("{timestamp}.{payload}").as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}
