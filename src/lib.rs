//! A small async Discord webhook wrapper.
//!
//! ```no_run
//! use ds_webhook::{DiscordWebhook, EditMessage, Embed, MessageBuilder};
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
//! hook.edit_message(EditMessage::new().content("Hello again!"), sent.message_id).await?;
//! hook.delete_message(sent.message_id).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Retries and rate limits
//! By default nothing is retried. With [`DiscordWebhook::with_max_retries`] a
//! request is retried after HTTP 429 (waiting as long as Discord asks) and after
//! HTTP 502/503/504 (short backoff). Rate-limit waits are shared by all clones
//! of a [`DiscordWebhook`], so parallel tasks don't all run into the same limit.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use reqwest::{Client, RequestBuilder, Response, StatusCode, Url};
use serde::{Deserialize, Serialize, Serializer, de::DeserializeOwned};
use thiserror::Error;

/// Result type used by every fallible function in this crate.
pub type Result<T> = std::result::Result<T, WebhookError>;

/// Default timeout for each HTTP request.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Retries used by [`DiscordWebhook::new`] and other URL checks, independent of
/// [`DiscordWebhook::with_max_retries`].
const CHECK_RETRIES: u32 = 3;

/// Errors returned by this crate.
#[derive(Error, Debug)]
pub enum WebhookError {
    /// The webhook URL could not be parsed or was rejected by Discord.
    #[error("invalid webhook: {0}")]
    InvalidWebhook(String),
    /// Transport error (including timeouts) or decoding error from reqwest.
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    /// Discord answered with a non-success status (other than 429).
    #[error("Discord API returned HTTP {status}: {body}")]
    Api {
        /// HTTP status code.
        status: u16,
        /// Response body, usually Discord's JSON error.
        body: String,
    },
    /// Discord rate limited the request and no retries were left.
    #[error("rate limited, retry after {retry_after}s (global: {global})")]
    RateLimited {
        /// Seconds to wait before retrying.
        retry_after: f64,
        /// Whether this is the global rate limit.
        global: bool,
    },
    /// Discord answered successfully, but the body was not what was expected.
    #[error("unexpected response from Discord: {0}")]
    InvalidResponse(String),
    /// The message would be rejected by Discord, e.g. a forum post without a name.
    #[error("invalid message: {0}")]
    InvalidMessage(String),
    /// A hex color string could not be parsed.
    #[error("invalid hex color: {0:?}")]
    InvalidColor(String),
    /// Avatar bytes are not a PNG, JPEG, GIF or WebP image.
    #[error("invalid avatar image: {0}")]
    InvalidImage(String),
}

/// Message object returned by Discord (only the fields this crate uses).
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct WebhookWaitResp {
    /// Message ID as Discord sends it (a string).
    pub id: Option<String>,
    /// Channel ID; for thread messages this is the thread ID.
    pub channel_id: Option<String>,
}

/// Body Discord sends with an HTTP 429 response.
#[derive(Debug, Serialize, Deserialize, PartialEq, PartialOrd, Clone, Default)]
pub struct WebhookRateLimitResp {
    /// Human-readable message from Discord.
    #[serde(default)]
    pub message: String,
    /// Seconds to wait before retrying.
    #[serde(default)]
    pub retry_after: f64,
    /// Whether this is the global rate limit rather than this webhook's own.
    #[serde(default)]
    pub global: bool,
}

/// IDs of a message that was sent or edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SentMessage {
    /// ID of the message.
    pub message_id: u64,
    /// Channel the message is in. For thread and forum messages this is the thread ID.
    pub channel_id: u64,
}

/// A new message, for [`DiscordWebhook::send_message`] and [`DiscordWebhook::send_to_thread`].
///
/// To create a forum post, wrap it in a [`ForumPost`]. To edit, use [`EditMessage`].
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct MessageBuilder {
    /// Text content, up to 2000 characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Overrides the webhook's display name for this message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Overrides the webhook's avatar for this message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// Embeds, up to 10.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embeds: Option<Vec<Embed>>,
}

impl MessageBuilder {
    /// Creates an empty message.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the text content.
    pub fn content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// Overrides the webhook's display name for this message.
    pub fn username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Overrides the webhook's avatar for this message.
    pub fn avatar_url(mut self, url: impl Into<String>) -> Self {
        self.avatar_url = Some(url.into());
        self
    }

    /// Adds an embed. Discord allows up to 10.
    pub fn embed(mut self, embed: Embed) -> Self {
        self.embeds.get_or_insert_with(Vec::new).push(embed);
        self
    }
}

/// A new forum post, for [`DiscordWebhook::create_thread`].
///
/// ```
/// use ds_webhook::{ForumPost, MessageBuilder};
///
/// let post = ForumPost::new("Release v2.0.0", MessageBuilder::new().content("Notes..."))
///     .tag(1234567890u64);
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct ForumPost {
    /// Post title, 1-100 characters.
    pub thread_name: String,
    /// Forum tag IDs to apply. Some forums require at least one.
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "ids_as_strings"
    )]
    pub applied_tags: Vec<u64>,
    /// The first message of the post.
    #[serde(flatten)]
    pub message: MessageBuilder,
}

impl ForumPost {
    /// A post with this title and first message.
    pub fn new(name: impl Into<String>, message: MessageBuilder) -> Self {
        Self {
            thread_name: name.into(),
            applied_tags: Vec::new(),
            message,
        }
    }

    /// Applies a forum tag.
    pub fn tag(mut self, tag_id: impl Into<u64>) -> Self {
        self.applied_tags.push(tag_id.into());
        self
    }

    /// Applies several forum tags.
    pub fn tags<I: IntoIterator<Item = u64>>(mut self, tag_ids: I) -> Self {
        self.applied_tags.extend(tag_ids);
        self
    }

    fn validate(&self) -> Result<()> {
        let len = self.thread_name.trim().chars().count();
        if len == 0 || len > 100 {
            return Err(WebhookError::InvalidMessage(format!(
                "forum post name must be 1-100 characters, got {len}"
            )));
        }
        Ok(())
    }
}

fn ids_as_strings<S: Serializer>(ids: &[u64], s: S) -> std::result::Result<S::Ok, S::Error> {
    s.collect_seq(ids.iter().map(u64::to_string))
}

/// Changes to an existing message, for [`DiscordWebhook::edit_message`] and
/// [`DiscordWebhook::edit_thread_msg`].
///
/// Anything not set stays as it is. To remove something, use
/// [`clear_content`](Self::clear_content) or [`clear_embeds`](Self::clear_embeds).
/// A message's name and avatar can't be changed after sending.
///
/// ```
/// use ds_webhook::{EditMessage, Embed};
///
/// // Replace the embeds, keep the text:
/// let edit = EditMessage::new().embed(Embed::new().title("Done"));
/// // Remove the text, keep the embeds:
/// let edit = EditMessage::new().clear_content();
/// ```
#[derive(Default, Debug, Clone, Serialize)]
pub struct EditMessage {
    /// New text content. `Some("")` removes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// New embeds, replacing all existing ones. `Some(vec![])` removes them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embeds: Option<Vec<Embed>>,
}

impl EditMessage {
    /// No changes yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the text content.
    pub fn content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// Removes the text content.
    pub fn clear_content(mut self) -> Self {
        self.content = Some(String::new());
        self
    }

    /// Adds an embed to the new set of embeds, which replaces all existing ones.
    pub fn embed(mut self, embed: Embed) -> Self {
        self.embeds.get_or_insert_with(Vec::new).push(embed);
        self
    }

    /// Removes all embeds.
    pub fn clear_embeds(mut self) -> Self {
        self.embeds = Some(Vec::new());
        self
    }
}

/// A Discord embed.
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Embed {
    /// Title, up to 256 characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Main text, up to 4096 characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Makes the title a link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// ISO8601 timestamp, e.g. `"2026-11-25T14:30:00Z"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// RGB color as an integer, see [`color_from_hex`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
    /// Small text at the bottom.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footer: Option<Footer>,
    /// Large image below the description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Image>,
    /// Small image in the top-right corner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<Image>,
    /// Name (and icon) shown above the title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    /// Name/value fields, up to 25.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<Field>>,
}

impl Embed {
    /// Creates an empty embed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the title (up to 256 characters).
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Sets the main text (up to 4096 characters).
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Makes the title a link to `url`.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// ISO8601 timestamp, e.g. `"2026-11-25T14:30:00Z"`.
    pub fn timestamp(mut self, timestamp: impl Into<String>) -> Self {
        self.timestamp = Some(timestamp.into());
        self
    }

    /// Sets the color of the side bar. Use [`color_from_hex`] for hex strings.
    pub fn color(mut self, color: u32) -> Self {
        self.color = Some(color);
        self
    }

    /// Sets the footer.
    pub fn footer(mut self, footer: Footer) -> Self {
        self.footer = Some(footer);
        self
    }

    /// Sets the large image.
    pub fn image(mut self, url: impl Into<String>) -> Self {
        self.image = Some(Image { url: url.into() });
        self
    }

    /// Sets the small top-right image.
    pub fn thumbnail(mut self, url: impl Into<String>) -> Self {
        self.thumbnail = Some(Image { url: url.into() });
        self
    }

    /// Sets the author line.
    pub fn author(mut self, author: Author) -> Self {
        self.author = Some(author);
        self
    }

    /// Adds a field. Inline fields sit side by side, up to three per row.
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

/// An image, referenced by URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    /// Image URL.
    pub url: String,
}

/// Embed footer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Footer {
    /// Footer text, up to 2048 characters.
    pub text: String,
    /// Small icon next to the footer text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

/// Embed author line, shown above the title.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    /// Author name, up to 256 characters.
    pub name: String,
    /// Makes the author name a link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Small icon next to the author name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

/// A name/value pair in an embed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Field {
    /// Field name, up to 256 characters.
    pub name: String,
    /// Field value, up to 1024 characters.
    pub value: String,
    /// Whether the field sits next to other inline fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline: Option<bool>,
}

impl Field {
    /// Creates a field.
    pub fn new(name: impl Into<String>, value: impl Into<String>, inline: bool) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            inline: Some(inline),
        }
    }
}

/// The webhook's own settings, as returned by Discord.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookInfo {
    /// Webhook ID.
    pub id: String,
    /// Default display name.
    pub name: Option<String>,
    /// Avatar hash, or `None` for Discord's default avatar.
    pub avatar: Option<String>,
    /// Channel the webhook posts to.
    pub channel_id: Option<String>,
    /// Server the webhook belongs to.
    pub guild_id: Option<String>,
}

/// Changes to the webhook's default name and avatar, for [`DiscordWebhook::modify`].
///
/// Anything not set stays as it is.
///
/// ```no_run
/// # async fn run(hook: ds_webhook::DiscordWebhook) -> ds_webhook::Result<()> {
/// use ds_webhook::ModifyWebhook;
///
/// let png = std::fs::read("avatar.png").unwrap();
/// hook.modify(ModifyWebhook::new().name("Deploy Bot").avatar(png)?).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Default, Serialize)]
pub struct ModifyWebhook {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// `None` = keep, `Some(None)` = remove (sent as `null`), `Some(Some(uri))` = set.
    #[serde(skip_serializing_if = "Option::is_none")]
    avatar: Option<Option<String>>,
}

impl ModifyWebhook {
    /// No changes yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the default display name (1-80 characters; Discord rejects names
    /// containing "discord" or "clyde").
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the default avatar from raw image bytes (PNG, JPEG, GIF or WebP).
    ///
    /// Returns [`WebhookError::InvalidImage`] if the bytes are not one of those formats.
    pub fn avatar(mut self, image: impl AsRef<[u8]>) -> Result<Self> {
        let bytes = image.as_ref();
        let mime = image_mime(bytes).ok_or_else(|| {
            WebhookError::InvalidImage("expected PNG, JPEG, GIF or WebP data".to_owned())
        })?;
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        self.avatar = Some(Some(format!("data:{mime};base64,{data}")));
        Ok(self)
    }

    /// Resets the avatar to Discord's default.
    pub fn remove_avatar(mut self) -> Self {
        self.avatar = Some(None);
        self
    }
}

/// Detects the image type from its first bytes.
fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
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
        channel_id: parse_id(resp.channel_id, "channel id")?,
    })
}

/// "Don't send before this instant", shared by all clones of a client.
#[derive(Clone, Default)]
struct RateGate(Arc<Mutex<Option<Instant>>>);

impl RateGate {
    fn remaining(&self) -> Option<Duration> {
        let until = *self.0.lock().unwrap_or_else(|e| e.into_inner());
        until.and_then(|t| t.checked_duration_since(Instant::now()))
    }

    fn block_for(&self, wait: Duration) {
        let until = Instant::now() + wait;
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none_or(|t| t < until) {
            *guard = Some(until);
        }
    }
}

/// Client for one Discord webhook.
///
/// Cloning is cheap: clones share the HTTP client and rate-limit state.
#[derive(Clone)]
pub struct DiscordWebhook {
    webhook_url: Url,
    thread_webhook_url: Option<Url>,
    client: Client,
    max_retries: u32,
    main_gate: RateGate,
    thread_gate: RateGate,
}

impl DiscordWebhook {
    /// Parses the URL and checks with Discord that the webhook exists.
    ///
    /// The check itself retries up to 3 times on rate limits and 5xx errors.
    /// Use [`from_url`](Self::from_url) to skip it.
    pub async fn new(url: &str) -> Result<Self> {
        let hook = Self::from_url(url)?;
        hook.check(&hook.webhook_url, &hook.main_gate).await?;
        Ok(hook)
    }

    /// Parses the URL without contacting Discord. Call
    /// [`get_info`](Self::get_info) later if you want to check it.
    pub fn from_url(url: &str) -> Result<Self> {
        Ok(Self {
            webhook_url: parse_url(url)?,
            thread_webhook_url: None,
            client: Client::builder().timeout(DEFAULT_TIMEOUT).build()?,
            max_retries: 0,
            main_gate: RateGate::default(),
            thread_gate: RateGate::default(),
        })
    }

    /// Uses a separate webhook (for example one in a forum channel) for thread
    /// calls, after checking with Discord that it exists.
    pub async fn with_thread_url(self, url: &str) -> Result<Self> {
        let hook = self.with_thread_url_unchecked(url)?;
        hook.check(hook.thread_url(), &hook.thread_gate).await?;
        Ok(hook)
    }

    /// Like [`with_thread_url`](Self::with_thread_url), without contacting Discord.
    pub fn with_thread_url_unchecked(mut self, url: &str) -> Result<Self> {
        self.thread_webhook_url = Some(parse_url(url)?);
        Ok(self)
    }

    /// Uses your own `reqwest::Client`, e.g. to share it or change the timeout
    /// (default [`DEFAULT_TIMEOUT`]).
    pub fn with_client(mut self, client: Client) -> Self {
        self.client = client;
        self
    }

    /// How many times to retry after HTTP 429 (rate limited) or 502/503/504.
    ///
    /// Default is 0: errors are returned right away, including
    /// [`WebhookError::RateLimited`] while an earlier rate limit is still running.
    ///
    /// Retrying a send after a 502/504 can very rarely post the message twice,
    /// if Discord handled it but the answer got lost.
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// The main webhook URL.
    pub fn webhook_url(&self) -> &Url {
        &self.webhook_url
    }

    /// The URL used for thread calls (falls back to the main webhook URL).
    pub fn thread_url(&self) -> &Url {
        self.thread_webhook_url
            .as_ref()
            .unwrap_or(&self.webhook_url)
    }

    /// How many times a failed request is retried.
    pub fn max_retries(&self) -> u32 {
        self.max_retries
    }

    /// Fetches the webhook's settings (name, avatar, channel, server).
    ///
    /// Returns [`WebhookError::InvalidWebhook`] if Discord doesn't know the webhook.
    pub async fn get_info(&self) -> Result<WebhookInfo> {
        let req = self.client.get(self.webhook_url.clone());
        let resp = self
            .send_with(req, &self.main_gate, self.max_retries)
            .await
            .map_err(rejected_webhook)?;
        Ok(resp.json().await?)
    }

    /// Changes the webhook's default name and/or avatar, and returns its new settings.
    ///
    /// This changes the main webhook (not the one set with
    /// [`with_thread_url`](Self::with_thread_url)) for every future message.
    /// To change the name or avatar for one message only, use
    /// [`MessageBuilder::username`] / [`MessageBuilder::avatar_url`] instead.
    pub async fn modify(&self, changes: ModifyWebhook) -> Result<WebhookInfo> {
        let req = self.client.patch(self.webhook_url.clone()).json(&changes);
        self.execute(req, &self.main_gate).await
    }

    /// Sends a message and returns its IDs.
    pub async fn send_message(&self, message: MessageBuilder) -> Result<SentMessage> {
        let req = self
            .client
            .post(self.webhook_url.clone())
            .query(&[("wait", "true")])
            .json(&message);
        to_sent(self.execute(req, &self.main_gate).await?)
    }

    /// Edits a message sent by this webhook.
    pub async fn edit_message(&self, edit: EditMessage, msg_id: u64) -> Result<SentMessage> {
        let req = self
            .client
            .patch(message_url(&self.webhook_url, msg_id))
            .json(&edit);
        to_sent(self.execute(req, &self.main_gate).await?)
    }

    /// Deletes a message sent by this webhook.
    pub async fn delete_message(&self, msg_id: u64) -> Result<()> {
        let req = self.client.delete(message_url(&self.webhook_url, msg_id));
        self.send(req, &self.main_gate).await.map(drop)
    }

    /// Creates a new forum post. `channel_id` in the result is the new thread's ID.
    ///
    /// Returns [`WebhookError::InvalidMessage`] without contacting Discord if the
    /// post name is empty or longer than 100 characters.
    pub async fn create_thread(&self, post: ForumPost) -> Result<SentMessage> {
        post.validate()?;
        let req = self
            .client
            .post(self.thread_url().clone())
            .query(&[("wait", "true")])
            .json(&post);
        to_sent(self.execute(req, self.thread_gate()).await?)
    }

    /// Posts a message into an existing thread or forum post.
    pub async fn send_to_thread(
        &self,
        message: MessageBuilder,
        thread_id: u64,
    ) -> Result<SentMessage> {
        let req = self
            .client
            .post(self.thread_url().clone())
            .query(&[("wait", "true")])
            .query(&[("thread_id", thread_id)])
            .json(&message);
        to_sent(self.execute(req, self.thread_gate()).await?)
    }

    /// Edits a message inside a thread.
    pub async fn edit_thread_msg(
        &self,
        edit: EditMessage,
        thread_id: u64,
        msg_id: u64,
    ) -> Result<SentMessage> {
        let req = self
            .client
            .patch(message_url(self.thread_url(), msg_id))
            .query(&[("thread_id", thread_id)])
            .json(&edit);
        to_sent(self.execute(req, self.thread_gate()).await?)
    }

    /// Deletes a message inside a thread.
    pub async fn delete_thread_msg(&self, thread_id: u64, msg_id: u64) -> Result<()> {
        let req = self
            .client
            .delete(message_url(self.thread_url(), msg_id))
            .query(&[("thread_id", thread_id)]);
        self.send(req, self.thread_gate()).await.map(drop)
    }

    /// Rate-limit state for thread calls: separate only if a separate webhook is used.
    fn thread_gate(&self) -> &RateGate {
        if self.thread_webhook_url.is_some() {
            &self.thread_gate
        } else {
            &self.main_gate
        }
    }

    /// Checks that Discord knows the webhook at `url`.
    async fn check(&self, url: &Url, gate: &RateGate) -> Result<()> {
        let req = self.client.get(url.clone());
        self.send_with(req, gate, CHECK_RETRIES)
            .await
            .map(drop)
            .map_err(rejected_webhook)
    }

    async fn execute<T: DeserializeOwned>(
        &self,
        req: RequestBuilder,
        gate: &RateGate,
    ) -> Result<T> {
        Ok(self.send(req, gate).await?.json::<T>().await?)
    }

    async fn send(&self, req: RequestBuilder, gate: &RateGate) -> Result<Response> {
        self.send_with(req, gate, self.max_retries).await
    }

    /// Sends the request, waiting for / retrying rate limits and 5xx errors up to
    /// `retries` times, and maps non-2xx answers to errors.
    async fn send_with(
        &self,
        req: RequestBuilder,
        gate: &RateGate,
        retries: u32,
    ) -> Result<Response> {
        let mut attempt = 0;
        loop {
            if let Some(wait) = gate.remaining() {
                if retries == 0 {
                    return Err(WebhookError::RateLimited {
                        retry_after: wait.as_secs_f64(),
                        global: false,
                    });
                }
                tokio::time::sleep(wait).await;
            }

            let this_try = req.try_clone().ok_or_else(|| {
                WebhookError::InvalidResponse("request body cannot be retried".to_owned())
            })?;
            let resp = this_try.send().await?;
            let status = resp.status();

            // Discord says no requests are left in this window: wait before the next one.
            if header(&resp, "x-ratelimit-remaining").as_deref() == Some("0")
                && let Some(secs) = header_secs(&resp, "x-ratelimit-reset-after")
            {
                gate.block_for(Duration::from_secs_f64(secs));
            }

            if status == StatusCode::TOO_MANY_REQUESTS {
                let header_retry = header_secs(&resp, "retry-after");
                let body = resp.text().await.unwrap_or_default();
                let mut info: WebhookRateLimitResp =
                    serde_json::from_str(&body).unwrap_or_default();
                if info.retry_after <= 0.0 {
                    info.retry_after = header_retry.unwrap_or(1.0);
                }
                gate.block_for(Duration::from_secs_f64(info.retry_after));

                if attempt < retries {
                    attempt += 1;
                    continue; // the gate makes the next round wait
                }
                return Err(WebhookError::RateLimited {
                    retry_after: info.retry_after,
                    global: info.global,
                });
            }

            if matches!(status.as_u16(), 502..=504) && attempt < retries {
                attempt += 1;
                tokio::time::sleep(backoff(attempt)).await;
                continue;
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

/// 0.5s, 1s, 2s, 4s, then 8s for every further attempt.
fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(500 << attempt.saturating_sub(1).min(4))
}

fn header(resp: &Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

fn header_secs(resp: &Response, name: &str) -> Option<f64> {
    header(resp, name)?
        .parse::<f64>()
        .ok()
        .filter(|s| s.is_finite() && *s >= 0.0)
}

/// Turns "Discord says no" (401/403/404) on a webhook URL into `InvalidWebhook`.
fn rejected_webhook(err: WebhookError) -> WebhookError {
    match err {
        WebhookError::Api {
            status: status @ (401 | 403 | 404),
            ..
        } => WebhookError::InvalidWebhook(format!(
            "Discord rejected the webhook URL (HTTP {status})"
        )),
        other => other,
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
    use serde_json::json;

    #[test]
    fn color_parsing() {
        assert_eq!(color_from_hex("ff8800").unwrap(), 0xff8800);
        assert_eq!(color_from_hex("#FF8800").unwrap(), 0xff8800);
        for bad in ["xyz", "", "1234567"] {
            assert!(matches!(
                color_from_hex(bad),
                Err(WebhookError::InvalidColor(_))
            ));
        }
    }

    #[test]
    fn none_fields_are_not_serialized() {
        let msg = MessageBuilder::new().content("hi");
        assert_eq!(
            serde_json::to_value(&msg).unwrap(),
            json!({ "content": "hi" })
        );
    }

    #[test]
    fn forum_post_serializes_name_tags_and_message() {
        let post = ForumPost::new("t", MessageBuilder::new().content("x").username("Bot"))
            .tag(5u64)
            .tags([6]);
        assert_eq!(
            serde_json::to_value(&post).unwrap(),
            json!({
                "thread_name": "t",
                "applied_tags": ["5", "6"],
                "content": "x",
                "username": "Bot"
            })
        );
        let untagged = ForumPost::new("t", MessageBuilder::new());
        assert_eq!(
            serde_json::to_value(&untagged).unwrap(),
            json!({ "thread_name": "t" })
        );
    }

    #[test]
    fn forum_post_name_is_validated() {
        assert!(
            ForumPost::new("ok", MessageBuilder::new())
                .validate()
                .is_ok()
        );
        assert!(
            ForumPost::new(" ", MessageBuilder::new())
                .validate()
                .is_err()
        );
        assert!(
            ForumPost::new("x".repeat(101), MessageBuilder::new())
                .validate()
                .is_err()
        );
    }

    #[test]
    fn edit_clear_sends_empty_values() {
        let edit = EditMessage::new().clear_content().clear_embeds();
        assert_eq!(
            serde_json::to_value(&edit).unwrap(),
            json!({ "content": "", "embeds": [] })
        );
        assert_eq!(serde_json::to_value(EditMessage::new()).unwrap(), json!({}));
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
    fn image_types() {
        assert_eq!(image_mime(b"\x89PNG\r\n"), Some("image/png"));
        assert_eq!(image_mime(b"\xFF\xD8\xFF\xE0"), Some("image/jpeg"));
        assert_eq!(image_mime(b"GIF89a"), Some("image/gif"));
        assert_eq!(image_mime(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(image_mime(b"RIFF\0\0\0\0WAVE"), None);
        assert_eq!(image_mime(b""), None);
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff(1), Duration::from_millis(500));
        assert_eq!(backoff(2), Duration::from_secs(1));
        assert_eq!(backoff(5), Duration::from_secs(8));
        assert_eq!(backoff(50), Duration::from_secs(8));
    }

    #[test]
    fn gate_keeps_the_later_deadline() {
        let gate = RateGate::default();
        assert!(gate.remaining().is_none());
        gate.block_for(Duration::from_secs(10));
        gate.block_for(Duration::from_secs(1));
        assert!(gate.remaining().unwrap() > Duration::from_secs(5));
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
