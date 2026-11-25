# TODO

## Fixes to existing methods
- [x] Default request timeout (about 30s)
- [x] `new()`: retry on 429 at startup, keep the webhook info, add `from_url()` (no network) and `get_info()` (info is returned by `get_info()`, not stored)
- [x] `create_thread`: forum `applied_tags`, plus a clear error when the thread name is missing
- [x] Retry on Discord 502/503/504 (when `with_max_retries` is on)
- [x] Document how to clear content and embeds on edit
- [x] Separate types for sending, forum posts and edits, so ignored fields can't be set
- [x] Rename `SentMessage.thread_id` → `channel_id`
- [x] Check `with_thread_url` with Discord (plus an unchecked variant)
- [x] Share the rate-limit wait across clones, and wait before hitting the limit using Discord's rate-limit headers

## Embed builder
- [x] `.footer(text)` / `.footer_with_icon(text, url)` shortcuts
- [x] `.author(name)` / `.author_with(name, url, icon)` shortcuts
- [x] `.timestamp_now()`, `.timestamp_unix(secs)`, and taking chrono dates
- [x] `.color_hex("#57F287")` and named Discord colors
- [x] `validate()` for Discord's size limits (title 256, description 4096, 25 fields, 6000 characters in total)
- [x] Remove, replace or clear fields

## Missing webhook features
- [x] File attachments (multipart upload, `attachment://` in embeds, keep or remove on edit)
- [x] Image grid from attachments: up to 10 uploaded images per message, shown as a gallery under the text
- [x] Media gallery component (Components V2): up to 10 images in one gallery block; check webhook rules first (non-interactive components on plain webhooks need a flag)
- [x] `allowed_mentions`
- [x] Message flags (silent messages, hide link previews)
- [x] Get a message by ID
- [x] Text-to-speech
- [x] Polls (question up to 300 characters, up to 10 answers of 55 characters, up to 32 days; can't be edited after sending)
- [x] Sends that don't wait for Discord's reply
- [x] Check message limits before sending (2000 characters, 10 embeds)
- [ ] Link buttons only (style 5, open a URL): test on a real plain webhook first, since Discord's docs don't say whether they count as non-interactive

## Repo and release
- [x] Commit the current work on `develop/tokio`
- [ ] Merge into `tokio` and push, so the docs workflow publishes ds-webhook
- [ ] Bump the action versions in both docs workflows to clear the Node 20 warning
- [ ] Optionally publish both crates to crates.io (docs.rs then hosts the docs)

## ds-format integration
- [x] Optional `ds-format` feature in ds-webhook (`.timestamp(Timestamp)` on embeds)
