//! Sends Web Push notifications
//!
//! VAPID is defined by RFC 8292. <https://www.rfc-editor.org/rfc/rfc8292>

mod config;
mod encrypt;
mod events;
mod send;
mod vapid;

use std::error::Error;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use p256::SecretKey;
use reqwest::Client;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

use crate::config::Config;
use crate::events::{Payload, ServerEvent, SidecarEvent};
use crate::send::Outcome;

/// Seconds to wait before connecting to Xenon again
const RECONNECT_SECONDS: u64 = 5;

#[tokio::main]
async fn main() {
    let config = match config::read(config::CONFIG_FILE) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("{}: {e}", config::CONFIG_FILE);
            return;
        }
    };

    let key = match vapid::read_or_create(vapid::KEY_FILE) {
        Ok(key) => key,
        Err(e) => {
            eprintln!("{}: {e}", vapid::KEY_FILE);
            return;
        }
    };

    let client = Client::new();

    loop {
        if let Err(e) = connect(&config, &key, &client).await {
            eprintln!("{}: {e}", config.xenon);
        }

        tokio::time::sleep(Duration::from_secs(RECONNECT_SECONDS)).await;
    }
}

// Helper Methods //

/// Connects to Xenon and sends what arrives, until the connection ends.
///
/// # Arguments
///
/// * `config` - Xenon's address, the shared secret, and the contact URI.
/// * `key` - The signing key.
/// * `client` - Shared HTTP client.
async fn connect(config: &Config, key: &SecretKey, client: &Client) -> Result<(), Box<dyn Error>> {

    let mut request = config.xenon.as_str().into_client_request()?;
    let secret = format!("Bearer {}", config.secret);
    request.headers_mut().insert("Authorization", secret.parse()?);

    let (mut socket, _) = tokio_tungstenite::connect_async(request).await?;

    // Xenon serves this to browsers, so it is reported before anything else
    let announce = SidecarEvent::Key { public_key: vapid::public_key_bytes(key) };
    socket.send(Message::Text(serde_json::to_string(&announce)?)).await?;

    println!("connected to {}, reported {}", config.xenon, vapid::public_key(key));

    while let Some(message) = socket.next().await {
        let Message::Text(text) = message? else {
            continue;
        };

        let event: ServerEvent = match serde_json::from_str(&text) {
            Ok(event) => event,
            Err(e) => {
                eprintln!("Xenon sent an event that did not parse: {e}");
                continue;
            }
        };

        deliver(config, key, client, event).await;
    }

    Ok(())
}

/// Sends one notification to every browser it names.
///
/// # Arguments
///
/// * `config` - Xenon's address, the shared secret, and the contact URI.
/// * `key` - The signing key.
/// * `client` - Shared HTTP client.
/// * `event` - What Xenon sent.
async fn deliver(config: &Config, key: &SecretKey, client: &Client, event: ServerEvent) {
    let ServerEvent::Push { room_id, room_name, author, body, renotify, subscriptions } = event;

    let payload = Payload {
        room_id,
        room: room_name,
        author,
        body,
        renotify
    };

    let text = match serde_json::to_string(&payload) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("could not build the push payload: {e}");
            return;
        }
    };

    for subscription in subscriptions {
        let result = send::send(
            client,
            key,
            &config.subject,
            &subscription.endpoint,
            &subscription.p256dh,
            &subscription.auth,
            text.as_bytes(),
            config.ttl
        ).await;

        match result {
            Ok(Outcome::Sent) => println!("{}: accepted", subscription.endpoint),
            Ok(outcome) => eprintln!("{}: {}", subscription.endpoint, describe(&outcome)),
            Err(e) => eprintln!("{}: {e}", subscription.endpoint)
        }
    }
}

/// What one push service answered, in words.
///
/// # Arguments
///
/// * `outcome` - What the push service answered.
fn describe(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Sent => "accepted".to_string(),
        Outcome::Gone => "the subscription no longer exists".to_string(),
        Outcome::Rejected => "signed with the wrong key".to_string(),
        Outcome::TooLarge => "the payload is over the service's limit".to_string(),
        Outcome::Throttled(seconds) => match seconds {
            Some(seconds) => format!("sending too fast, retry after {seconds}s"),
            None => "sending too fast".to_string()
        },
        Outcome::Other(status) => format!("answered {status}")
    }
}
