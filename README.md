# ds-webhook

A small async Discord webhook wrapper for Rust (tokio + reqwest).

## Usage

```toml
[dependencies]
ds-webhook = { git = "https://github.com/Saniee/ds-webhook", branch = "tokio" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust
use ds_webhook::{color_from_hex, DiscordWebhook, EditMessage, Embed, ForumPost, MessageBuilder};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Checks with Discord that the webhook exists (use from_url to skip the check).
    let hook = DiscordWebhook::new("https://discord.com/api/webhooks/ID/TOKEN")
        .await?
        .with_max_retries(2); // retry on rate limits and Discord 5xx errors

    let sent = hook
        .send_message(
            MessageBuilder::new().content("Deploy started").embed(
                Embed::new()
                    .title("Build #42")
                    .color(color_from_hex("#57F287")?)
                    .timestamp("2026-11-25T14:30:00Z")
                    .field("Branch", "main", true),
            ),
        )
        .await?;

    // Only what you set changes; the embed above stays.
    hook.edit_message(EditMessage::new().content("Deploy finished"), sent.message_id)
        .await?;
    hook.delete_message(sent.message_id).await?;

    // Forum channels: create a post, then reply in it.
    // Use .with_thread_url(...).await? if the forum has its own webhook.
    let post = hook
        .create_thread(
            ForumPost::new("Release notes", MessageBuilder::new().content("v2.0.0"))
                .tag(1234567890u64), // forum tag ID
        )
        .await?;
    hook.send_to_thread(MessageBuilder::new().content("Details..."), post.channel_id)
        .await?;

    Ok(())
}
```

## Message types

| Type | Used by | Holds |
| --- | --- | --- |
| `MessageBuilder` | `send_message`, `send_to_thread` | content, embeds, username, avatar_url |
| `ForumPost` | `create_thread` | post name (1–100 characters, checked locally), forum tags, and a `MessageBuilder` |
| `EditMessage` | `edit_message`, `edit_thread_msg` | content, embeds |

**Editing:** anything you don't set stays as it is. `clear_content()` removes the text, and `clear_embeds()` removes all embeds. Setting embeds replaces all existing ones. A message's name and avatar can't be changed after sending.

## Name and avatar

**For one message:** set `username` / `avatar_url` on the `MessageBuilder`, including inside a `ForumPost` or for `send_to_thread`.

```rust
MessageBuilder::new().content("Build passed").username("CI Bot").avatar_url("https://example.com/ci.png");
```

**For every message (the webhook's defaults):** use `modify`. Discord wants the avatar as image data, not a URL. Pass the raw bytes of a PNG, JPEG, GIF or WebP file.

```rust
use ds_webhook::ModifyWebhook;

let png = std::fs::read("avatar.png")?;
let info = hook.modify(ModifyWebhook::new().name("Deploy Bot").avatar(png)?).await?;
hook.modify(ModifyWebhook::new().remove_avatar()).await?; // back to Discord's default
```

`get_info()` returns the current name, avatar, channel and server.

## Connecting

| Call | Contacts Discord? |
| --- | --- |
| `DiscordWebhook::new(url).await?` | Yes, checks the webhook exists (retrying up to 3 times on rate limits / 5xx) |
| `DiscordWebhook::from_url(url)?` | No, only parses the URL |
| `.with_thread_url(url).await?` | Yes, checks the forum webhook |
| `.with_thread_url_unchecked(url)?` | No |
| `.get_info().await?` | Yes, returns the webhook's settings |

Every request has a 30-second timeout (`DEFAULT_TIMEOUT`). Use `.with_client(..)` to pass your own `reqwest::Client` with other settings.

## Retries and rate limits

`with_max_retries(n)` (default 0) retries a request up to `n` times:
- **HTTP 429:** waits as long as Discord asks, then retries.
- **HTTP 502/503/504:** retries after 0.5 s, 1 s, 2 s, … (capped at 8 s). In rare cases this can post a message twice, if Discord handled it but its answer was lost.

Rate limits are tracked per webhook and **shared by all clones** of a `DiscordWebhook`. After a 429, or when Discord reports that no requests are left in the current window, other calls wait (with retries) or return `RateLimited` (without retries) instead of sending and getting another 429.

## Errors

Every call returns `ds_webhook::Result<T>`, with a `WebhookError` that you can match on:

| Variant | When |
| --- | --- |
| `InvalidWebhook` | The URL can't be parsed, or Discord says the webhook doesn't exist (401/403/404 on a check) |
| `Api { status, body }` | Discord returned a non-2xx status other than 429 |
| `RateLimited { retry_after, global }` | Rate limited and no retries left |
| `InvalidMessage` | A forum post name is empty or longer than 100 characters |
| `InvalidResponse` | Discord's reply was missing or had malformed IDs |
| `InvalidColor` | `color_from_hex` got bad input |
| `InvalidImage` | `ModifyWebhook::avatar` got bytes that aren't PNG, JPEG, GIF or WebP |
| `Request` | Network error, timeout, or decoding error from reqwest |

`WebhookError` implements `std::error::Error`, so `?` works directly in `anyhow` apps.

## Migrating from 1.x

- `new(url, thread_url)` is now `new(url)`, plus `.with_thread_url(thread_url).await?` when you need it.
- `ThreadBuilder` is gone:
  - for `send_to_thread`, use `MessageBuilder`
  - for `create_thread`, use `ForumPost::new(name, message)`
  - for edits, use `EditMessage`
- `MessageBuilder.message` is renamed to `content`.
- Send and edit calls return `SentMessage { message_id, channel_id }` instead of tuples or `u64`. For thread messages, `channel_id` is the thread ID.
- Rate limits come back as `Err(WebhookError::RateLimited { .. })` instead of `SenderReturn.rate_limit`.
- `Embed.timestamp` is an ISO8601 `String`, `Embed.color` is `u32`, and `Field.inline` is `Option<bool>`.
- `hook.hex_to_dec(..)` is now the free function `color_from_hex(..)`, which returns a `Result` instead of panicking.
- The library no longer depends on `anyhow`.
