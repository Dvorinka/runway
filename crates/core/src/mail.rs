//! Outbound mail — SMTP via lettre. When no SMTP host is configured the
//! message is logged instead of sent (dev mode convenience; matches the
//! devpush dev experience).

use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::config::Settings;
use crate::error::{Error, Result};

pub fn smtp_configured(settings: &Settings) -> bool {
    settings.smtp_host.is_some() && settings.smtp_from.is_some()
}

pub async fn send(settings: &Settings, to: &str, subject: &str, body: &str) -> Result<()> {
    if !smtp_configured(settings) {
        tracing::info!(
            to,
            subject,
            body,
            "smtp not configured — mail logged, not sent"
        );
        return Ok(());
    }

    let message = Message::builder()
        .from(
            settings
                .smtp_from
                .as_deref()
                .unwrap()
                .parse()
                .map_err(|e| Error::Config(format!("SMTP_FROM invalid: {e}")))?,
        )
        .to(to
            .parse()
            .map_err(|e| Error::Config(format!("bad recipient: {e}")))?)
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body.to_string())
        .map_err(|e| Error::Other(e.into()))?;

    let host = settings.smtp_host.as_deref().unwrap();
    let transport = if settings.smtp_tls {
        let builder = if settings.smtp_port == 465 {
            AsyncSmtpTransport::<Tokio1Executor>::relay(host)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
        }
        .map_err(|e| Error::Other(e.into()))?;
        builder.port(settings.smtp_port)
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host).port(settings.smtp_port)
    };

    let transport = if let (Some(u), Some(p)) = (&settings.smtp_user, &settings.smtp_password) {
        transport.credentials(Credentials::new(u.clone(), p.clone()))
    } else {
        transport
    }
    .build();

    transport
        .send(message)
        .await
        .map_err(|e| Error::Other(e.into()))?;
    Ok(())
}
