use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct Email {
    pub to: String,
    pub subject: String,
    pub body: String,
}

#[async_trait]
pub trait EmailSender: Send + Sync + 'static {
    async fn send(&self, email: Email) -> anyhow::Result<()>;
}

/// Logs the email instead of sending it. Useful for dev + tests.
/// Swap for a real Resend/SES/SMTP impl by implementing `EmailSender`.
pub struct LogEmailSender;

#[async_trait]
impl EmailSender for LogEmailSender {
    async fn send(&self, email: Email) -> anyhow::Result<()> {
        tracing::info!(
            to = %email.to,
            subject = %email.subject,
            "email.send (log-only stub)"
        );
        Ok(())
    }
}
