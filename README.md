# ds-webhook

A small async Discord webhook wrapper for Rust (tokio + reqwest).

## Usage

```toml
[dependencies]
ds-webhook = { git = "https://github.com/Saniee/ds-webhook", branch = "tokio" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust
use ds_webhook::{color_from_hex, DiscordWebhook, Embed, MessageBuilder, ThreadBuilder};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Checks with Discord that the webhook exists.
    let hook = DiscordWebhook::new("https://discord.com/api/webhooks/ID/TOKEN")
        .await?
        .with_max_retries(2); // wait and retry on HTTP 429

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

    // Fields left as None are not sent, so they stay as they were.
    hook.edit_message(MessageBuilder::new().content("Deploy finished"), sent.message_id)
        .await?;
    hook.delete_message(sent.message_id).await?;

    // Forum channels: create a post, then reply in it.
    // Use .with_thread_url(...) if the forum has its own webhook.
    let post = hook
        .create_thread(ThreadBuilder::new().name("Release notes").content("v2.0.0"))
        .await?;
    hook.send_to_thread(ThreadBuilder::new().content("Details..."), post.thread_id)
        .await?;

    Ok(())
}
```

## Errors

Every call returns `ds_webhook::Result<T>`, with a `WebhookError` that you can match on:

| Variant | When |
| --- | --- |
| `InvalidWebhook` | The URL can't be parsed, or Discord rejected it in `new` |
| `Api { status, body }` | Discord returned a non-2xx status other than 429 |
| `RateLimited { retry_after, global }` | HTTP 429 and no retries left (`with_max_retries`, default 0) |
| `InvalidResponse` | Discord's reply was missing or had malformed ids |
| `InvalidColor` | `color_from_hex` got bad input |
| `Request` | Network or decoding error from reqwest |

`WebhookError` implements `std::error::Error`, so `?` works directly in `anyhow` apps.

## Migrating from 1.x

- `new(url, thread_url)` is now `new(url)`, plus `.with_thread_url(thread_url)?` when you need it.
- Send/edit calls return `SentMessage { message_id, thread_id }` instead of tuples or `u64`.
- Rate limits come back as `Err(WebhookError::RateLimited { .. })` instead of `SenderReturn.rate_limit`.
- `Embed.timestamp` is an ISO8601 `String`, `Embed.color` is `u32`, and `Field.inline` is `Option<bool>`.
- `hook.hex_to_dec(..)` is now the free function `color_from_hex(..)`, which returns a `Result` instead of panicking.
- The library no longer depends on `anyhow`.
