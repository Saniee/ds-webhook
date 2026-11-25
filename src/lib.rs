//! A small async Discord webhook wrapper.
//!
//! ```no_run
//! use ds_webhook::{DiscordWebhook, Embed, MessageBuilder};
//!
//! # async fn run() -> ds_webhook::Result<()> {
//! let hook = DiscordWebhook::new("https://discord.com/api/webhooks/ID/TOKEN").await?;
//! let sent = hook
//!     .send_message(
//!         MessageBuilder::new()
//!             .content("Hello!")
//!             .embed(Embed::new().title("Status").field("Uptime", "3d", true)),
//!     )
//!     .await?;
//! hook.delete_message(sent.message_id).await?;
//! # Ok(())
//! # }
//! ```

use std::time::Duration;

use reqwest::{Client, RequestBuilder, Response, StatusCode, Url};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

/// Result type used by every fallible function in this crate.
pub type Result<T> = std::result::Result<T, WebhookError>;

/// Errors returned by this crate.
#[derive(Error, Debug)]
pub enum WebhookError {
    /// The webhook URL could not be parsed or was rejected by Discord.
    #[error("invalid webhook: {0}")]
    InvalidWebhook(String),
    /// Transport or decoding error from reqwest.
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    /// Discord answered with a non-success status (other than 429).
    #[error("Discord API returned HTTP {status}: {body}")]
    Api { status: u16, body: String },
    /// Discord rate limited the request and no retries were left.
    #[error("rate limited, retry after {retry_after}s (global: {global})")]
    RateLimited { retry_after: f64, global: bool },
    /// Discord answered successfully, but the body was not what was expected.
    #[error("unexpected response from Discord: {0}")]
    InvalidResponse(String),
    /// A hex color string could not be parsed.
    #[error("invalid hex color: {0:?}")]
    InvalidColor(String),
}

/// Message object returned by Discord (only the fields this crate uses).
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct WebhookWaitResp {
    pub id: Option<String>,
    #[serde(rename = "channel_id")]
    pub thread_id: Option<String>,
}

/// Body Discord sends with an HTTP 429 response.
#[derive(Debug, Serialize, Deserialize, PartialEq, PartialOrd, Clone, Default)]
pub struct WebhookRateLimitResp {
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub retry_after: f64,
    #[serde(default)]
    pub global: bool,
}

/// IDs of a message that was sent or edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SentMessage {
    pub message_id: u64,
    /// Thread ID for thread calls, otherwise the channel ID the message lives in.
    pub thread_id: u64,
}

/// A normal webhook message.
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct MessageBuilder {
    #[serde(rename = "content", skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embeds: Option<Vec<Embed>>,
}

impl MessageBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn content(mut self, content: impl Into<String>) -> Self {
        self.message = Some(content.into());
        self
    }

    pub fn username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    pub fn avatar_url(mut self, url: impl Into<String>) -> Self {
        self.avatar_url = Some(url.into());
        self
    }

    pub fn embed(mut self, embed: Embed) -> Self {
        self.embeds.get_or_insert_with(Vec::new).push(embed);
        self
    }
}

/// A message for a thread or forum post.
///
/// Set `thread_name` and use [`DiscordWebhook::create_thread`] to create a new
/// forum post, or use [`DiscordWebhook::send_to_thread`] to post into an
/// existing thread.
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct ThreadBuilder {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_name: Option<String>,
    #[serde(rename = "content", skip_serializing_if = "Option::is_none")]
    pub thread_message: Option<String>,
    #[serde(rename = "embeds", skip_serializing_if = "Option::is_none")]
    pub thread_embeds: Option<Vec<Embed>>,
}

impl ThreadBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.thread_name = Some(name.into());
        self
    }

    pub fn content(mut self, content: impl Into<String>) -> Self {
        self.thread_message = Some(content.into());
        self
    }

    pub fn embed(mut self, embed: Embed) -> Self {
        self.thread_embeds.get_or_insert_with(Vec::new).push(embed);
        self
    }
}

/// A Discord embed.
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Embed {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// ISO8601 timestamp, e.g. `"2026-11-25T14:30:00Z"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// RGB color as an integer, see [`color_from_hex`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footer: Option<Footer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Image>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<Image>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<Field>>,
}

impl Embed {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// ISO8601 timestamp, e.g. `"2026-11-25T14:30:00Z"`.
    pub fn timestamp(mut self, timestamp: impl Into<String>) -> Self {
        self.timestamp = Some(timestamp.into());
        self
    }

    pub fn color(mut self, color: u32) -> Self {
        self.color = Some(color);
        self
    }

    pub fn footer(mut self, footer: Footer) -> Self {
        self.footer = Some(footer);
        self
    }

    pub fn image(mut self, url: impl Into<String>) -> Self {
        self.image = Some(Image { url: url.into() });
        self
    }

    pub fn thumbnail(mut self, url: impl Into<String>) -> Self {
        self.thumbnail = Some(Image { url: url.into() });
        self
    }

    pub fn author(mut self, author: Author) -> Self {
        self.author = Some(author);
        self
    }

    pub fn field(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
        inline: bool,
    ) -> Self {
        self.fields
            .get_or_insert_with(Vec::new)
            .push(Field::new(name, value, inline));
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Footer {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline: Option<bool>,
}

impl Field {
    pub fn new(name: impl Into<String>, value: impl Into<String>, inline: bool) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            inline: Some(inline),
        }
    }
}

/// Parses a hex color such as `"ff8800"` or `"#FF8800"` into an embed color.
pub fn color_from_hex(hex: &str) -> Result<u32> {
    let digits = hex.trim().trim_start_matches('#');
    if digits.is_empty() || digits.len() > 6 {
        return Err(WebhookError::InvalidColor(hex.to_owned()));
    }
    u32::from_str_radix(digits, 16).map_err(|_| WebhookError::InvalidColor(hex.to_owned()))
}

fn parse_id(id: Option<String>, what: &str) -> Result<u64> {
    let id = id.ok_or_else(|| WebhookError::InvalidResponse(format!("missing {what}")))?;
    id.parse::<u64>()
        .map_err(|_| WebhookError::InvalidResponse(format!("{what} is not a number: {id:?}")))
}

fn to_sent(resp: WebhookWaitResp) -> Result<SentMessage> {
    Ok(SentMessage {
        message_id: parse_id(resp.id, "message id")?,
        thread_id: parse_id(resp.thread_id, "channel id")?,
    })
}

/// Client for one Discord webhook.
///
/// Cloning is cheap; the underlying `reqwest::Client` is shared.
#[derive(Clone)]
pub struct DiscordWebhook {
    webhook_url: Url,
    thread_webhook_url: Option<Url>,
    client: Client,
    max_retries: u32,
}

impl DiscordWebhook {
    /// Parses the URL and checks with Discord that the webhook exists.
    ///
    /// The same URL is used for thread calls unless [`with_thread_url`](Self::with_thread_url) is set.
    pub async fn new(url: &str) -> Result<Self> {
        let webhook_url = parse_url(url)?;
        let hook = Self {
            webhook_url,
            thread_webhook_url: None,
            client: Client::new(),
            max_retries: 0,
        };

        match hook.send(hook.client.get(hook.webhook_url.clone())).await {
            Ok(_) => Ok(hook),
            Err(WebhookError::Api { status, .. }) => Err(WebhookError::InvalidWebhook(format!(
                "Discord rejected the webhook URL (HTTP {status})"
            ))),
            Err(e) => Err(e),
        }
    }

    /// Uses a separate webhook (for example one in a forum channel) for thread calls.
    /// This URL is not checked with Discord.
    pub fn with_thread_url(mut self, url: &str) -> Result<Self> {
        self.thread_webhook_url = Some(parse_url(url)?);
        Ok(self)
    }

    /// Uses your own `reqwest::Client`, e.g. to share it or set timeouts.
    pub fn with_client(mut self, client: Client) -> Self {
        self.client = client;
        self
    }

    /// How many times to wait and retry when Discord answers HTTP 429.
    /// Default is 0, which returns [`WebhookError::RateLimited`] right away.
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    pub fn webhook_url(&self) -> &Url {
        &self.webhook_url
    }

    /// The URL used for thread calls (falls back to the main webhook URL).
    pub fn thread_url(&self) -> &Url {
        self.thread_webhook_url
            .as_ref()
            .unwrap_or(&self.webhook_url)
    }

    pub fn max_retries(&self) -> u32 {
        self.max_retries
    }

    /// Creates a new forum post. `thread.thread_name` must be set.
    pub async fn create_thread(&self, thread: ThreadBuilder) -> Result<SentMessage> {
        let req = self
            .client
            .post(self.thread_url().clone())
            .query(&[("wait", "true")])
            .json(&thread);
        to_sent(self.execute(req).await?)
    }

    /// Posts a message into an existing thread.
    pub async fn send_to_thread(
        &self,
        thread: ThreadBuilder,
        thread_id: u64,
    ) -> Result<SentMessage> {
        let req = self
            .client
            .post(self.thread_url().clone())
            .query(&[("wait", "true")])
            .query(&[("thread_id", thread_id)])
            .json(&thread);
        to_sent(self.execute(req).await?)
    }

    /// Edits a message inside a thread.
    pub async fn edit_thread_msg(
        &self,
        thread: ThreadBuilder,
        thread_id: u64,
        msg_id: u64,
    ) -> Result<SentMessage> {
        let req = self
            .client
            .patch(message_url(self.thread_url(), msg_id))
            .query(&[("thread_id", thread_id)])
            .json(&thread);
        to_sent(self.execute(req).await?)
    }

    /// Deletes a message inside a thread.
    pub async fn delete_thread_msg(&self, thread_id: u64, msg_id: u64) -> Result<()> {
        let req = self
            .client
            .delete(message_url(self.thread_url(), msg_id))
            .query(&[("thread_id", thread_id)]);
        self.send(req).await.map(drop)
    }

    pub async fn send_message(&self, content: MessageBuilder) -> Result<SentMessage> {
        let req = self
            .client
            .post(self.webhook_url.clone())
            .query(&[("wait", "true")])
            .json(&content);
        to_sent(self.execute(req).await?)
    }

    /// Edits a message. Fields left as `None` are not sent, so they stay unchanged.
    pub async fn edit_message(&self, content: MessageBuilder, msg_id: u64) -> Result<SentMessage> {
        let req = self
            .client
            .patch(message_url(&self.webhook_url, msg_id))
            .json(&content);
        to_sent(self.execute(req).await?)
    }

    pub async fn delete_message(&self, msg_id: u64) -> Result<()> {
        let req = self.client.delete(message_url(&self.webhook_url, msg_id));
        self.send(req).await.map(drop)
    }

    async fn execute<T: DeserializeOwned>(&self, req: RequestBuilder) -> Result<T> {
        Ok(self.send(req).await?.json::<T>().await?)
    }

    /// Sends the request, handles 429 (with optional retries) and maps non-2xx to errors.
    async fn send(&self, req: RequestBuilder) -> Result<Response> {
        let mut attempt = 0;
        loop {
            let this_try = req.try_clone().ok_or_else(|| {
                WebhookError::InvalidResponse("request body cannot be retried".to_owned())
            })?;
            let resp = this_try.send().await?;
            let status = resp.status();

            if status == StatusCode::TOO_MANY_REQUESTS {
                let header_secs = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<f64>().ok());
                let body = resp.text().await.unwrap_or_default();
                let mut info: WebhookRateLimitResp =
                    serde_json::from_str(&body).unwrap_or_default();
                if info.retry_after <= 0.0 {
                    info.retry_after = header_secs.unwrap_or(1.0);
                }

                if attempt < self.max_retries {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_secs_f64(info.retry_after.max(0.0))).await;
                    continue;
                }
                return Err(WebhookError::RateLimited {
                    retry_after: info.retry_after,
                    global: info.global,
                });
            }

            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(WebhookError::Api {
                    status: status.as_u16(),
                    body,
                });
            }

            return Ok(resp);
        }
    }
}

fn parse_url(url: &str) -> Result<Url> {
    let parsed = Url::parse(url)
        .map_err(|e| WebhookError::InvalidWebhook(format!("not a valid URL ({e})")))?;
    if parsed.cannot_be_a_base() {
        return Err(WebhookError::InvalidWebhook("not a valid URL".to_owned()));
    }
    Ok(parsed)
}

fn message_url(base: &Url, msg_id: u64) -> Url {
    let mut url = base.clone();
    url.path_segments_mut()
        .expect("checked in parse_url")
        .pop_if_empty()
        .push("messages")
        .push(&msg_id.to_string());
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_parsing() {
        assert_eq!(color_from_hex("ff8800").unwrap(), 0xff8800);
        assert_eq!(color_from_hex("#FF8800").unwrap(), 0xff8800);
        assert!(matches!(
            color_from_hex("xyz"),
            Err(WebhookError::InvalidColor(_))
        ));
        assert!(matches!(
            color_from_hex(""),
            Err(WebhookError::InvalidColor(_))
        ));
        assert!(matches!(
            color_from_hex("1234567"),
            Err(WebhookError::InvalidColor(_))
        ));
    }

    #[test]
    fn none_fields_are_not_serialized() {
        let msg = MessageBuilder::new().content("hi");
        assert_eq!(
            serde_json::to_value(&msg).unwrap(),
            serde_json::json!({ "content": "hi" })
        );

        let thread = ThreadBuilder::new()
            .name("t")
            .embed(Embed::new().field("a", "b", true));
        assert_eq!(
            serde_json::to_value(&thread).unwrap(),
            serde_json::json!({
                "thread_name": "t",
                "embeds": [{ "fields": [{ "name": "a", "value": "b", "inline": true }] }]
            })
        );
    }

    #[test]
    fn id_parsing() {
        assert_eq!(parse_id(Some("123".into()), "id").unwrap(), 123);
        assert!(matches!(
            parse_id(None, "id"),
            Err(WebhookError::InvalidResponse(_))
        ));
        assert!(matches!(
            parse_id(Some("abc".into()), "id"),
            Err(WebhookError::InvalidResponse(_))
        ));
    }

    #[test]
    fn message_url_keeps_query_and_handles_trailing_slash() {
        let base = Url::parse("https://discord.com/api/webhooks/1/tok/?x=1").unwrap();
        assert_eq!(
            message_url(&base, 42).as_str(),
            "https://discord.com/api/webhooks/1/tok/messages/42?x=1"
        );
    }
}
