use std::time::{Duration, Instant};

use ds_webhook::{
    Attachment, DiscordWebhook, EditMessage, ForumPost, MessageBuilder, ModifyWebhook, WebhookError,
};
use serde_json::json;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const HOOK_PATH: &str = "/api/webhooks/1/token";
const FORUM_PATH: &str = "/api/webhooks/2/forum-token";

async fn setup() -> (MockServer, DiscordWebhook) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(HOOK_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "1" })))
        .mount(&server)
        .await;
    let hook = DiscordWebhook::new(&format!("{}{}", server.uri(), HOOK_PATH))
        .await
        .unwrap();
    (server, hook)
}

fn message(id: &str, channel: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "id": id, "channel_id": channel }))
}

fn rate_limited(secs: f64) -> ResponseTemplate {
    ResponseTemplate::new(429)
        .set_body_json(json!({ "message": "rate limited", "retry_after": secs, "global": false }))
}

// ---- constructors ----

#[tokio::test]
async fn new_rejects_unknown_webhook() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({ "message": "401: Unauthorized" })),
        )
        .mount(&server)
        .await;
    let err = DiscordWebhook::new(&format!("{}{}", server.uri(), HOOK_PATH))
        .await
        .err()
        .unwrap();
    assert!(matches!(err, WebhookError::InvalidWebhook(_)), "{err:?}");
}

#[tokio::test]
async fn new_survives_a_rate_limit_during_the_check() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(rate_limited(0.01))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "1" })))
        .expect(1)
        .mount(&server)
        .await;
    DiscordWebhook::new(&format!("{}{}", server.uri(), HOOK_PATH))
        .await
        .unwrap();
}

#[tokio::test]
async fn from_url_does_not_contact_discord_and_get_info_returns_settings() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(HOOK_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "1", "name": "Bot", "avatar": null, "channel_id": "20", "guild_id": "30",
            "type": 1, "token": "secret"
        })))
        .expect(1) // only get_info, not from_url
        .mount(&server)
        .await;

    let hook = DiscordWebhook::from_url(&format!("{}{}", server.uri(), HOOK_PATH)).unwrap();
    let info = hook.get_info().await.unwrap();
    assert_eq!(info.name.as_deref(), Some("Bot"));
    assert_eq!(info.channel_id.as_deref(), Some("20"));
}

#[test]
fn from_url_rejects_garbage() {
    assert!(matches!(
        DiscordWebhook::from_url("not a url"),
        Err(WebhookError::InvalidWebhook(_))
    ));
}

#[tokio::test]
async fn with_thread_url_checks_the_forum_webhook() {
    let (server, hook) = setup().await;
    Mock::given(method("GET"))
        .and(path(FORUM_PATH))
        .respond_with(ResponseTemplate::new(404).set_body_string("Unknown Webhook"))
        .mount(&server)
        .await;

    let err = hook
        .clone()
        .with_thread_url(&format!("{}{}", server.uri(), FORUM_PATH))
        .await
        .err()
        .unwrap();
    assert!(matches!(err, WebhookError::InvalidWebhook(_)), "{err:?}");

    // The unchecked variant only parses.
    hook.with_thread_url_unchecked(&format!("{}{}", server.uri(), FORUM_PATH))
        .unwrap();
}

// ---- messages ----

#[tokio::test]
async fn send_message_sends_only_set_fields() {
    let (server, hook) = setup().await;
    Mock::given(method("POST"))
        .and(path(HOOK_PATH))
        .and(query_param("wait", "true"))
        .and(body_json(json!({ "content": "hi" })))
        .respond_with(message("10", "20"))
        .expect(1)
        .mount(&server)
        .await;

    let sent = hook
        .send_message(MessageBuilder::new().content("hi"))
        .await
        .unwrap();
    assert_eq!((sent.message_id, sent.channel_id), (10, 20));
}

#[tokio::test]
async fn edit_sends_only_content_and_embeds() {
    let (server, hook) = setup().await;
    Mock::given(method("PATCH"))
        .and(path(format!("{HOOK_PATH}/messages/10")))
        .and(body_json(json!({ "content": "", "embeds": [] })))
        .respond_with(message("10", "20"))
        .expect(1)
        .mount(&server)
        .await;

    hook.edit_message(EditMessage::new().clear_content().clear_embeds(), 10)
        .await
        .unwrap();
}

#[tokio::test]
async fn api_error_keeps_status_and_body() {
    let (server, hook) = setup().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_string("Cannot send an empty message"))
        .mount(&server)
        .await;

    match hook.send_message(MessageBuilder::new()).await {
        Err(WebhookError::Api { status, body }) => {
            assert_eq!(status, 400);
            assert!(body.contains("empty message"));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn delete_message_maps_404() {
    let (server, hook) = setup().await;
    Mock::given(method("DELETE"))
        .and(path(format!("{HOOK_PATH}/messages/5")))
        .respond_with(ResponseTemplate::new(404).set_body_string("Unknown Message"))
        .mount(&server)
        .await;

    assert!(matches!(
        hook.delete_message(5).await,
        Err(WebhookError::Api { status: 404, .. })
    ));
}

// ---- threads and forum posts ----

#[tokio::test]
async fn create_thread_sends_name_tags_and_message() {
    let (server, hook) = setup().await;
    Mock::given(method("POST"))
        .and(path(HOOK_PATH))
        .and(query_param("wait", "true"))
        .and(body_json(json!({
            "thread_name": "Release v2",
            "applied_tags": ["111", "222"],
            "content": "notes",
            "username": "Release Bot"
        })))
        .respond_with(message("30", "40"))
        .expect(1)
        .mount(&server)
        .await;

    let post = ForumPost::new(
        "Release v2",
        MessageBuilder::new()
            .content("notes")
            .username("Release Bot"),
    )
    .tags([111, 222]);
    let sent = hook.create_thread(post).await.unwrap();
    assert_eq!((sent.message_id, sent.channel_id), (30, 40));
}

#[tokio::test]
async fn create_thread_rejects_empty_name_locally() {
    let (server, hook) = setup().await;
    Mock::given(method("POST"))
        .respond_with(message("1", "1"))
        .expect(0)
        .mount(&server)
        .await;

    let err = hook
        .create_thread(ForumPost::new("  ", MessageBuilder::new().content("x")))
        .await
        .err()
        .unwrap();
    assert!(matches!(err, WebhookError::InvalidMessage(_)), "{err:?}");
}

#[tokio::test]
async fn thread_calls_use_thread_id_and_message_path() {
    let (server, hook) = setup().await;
    Mock::given(method("POST"))
        .and(path(HOOK_PATH))
        .and(query_param("wait", "true"))
        .and(query_param("thread_id", "99"))
        .and(body_json(json!({
            "content": "a",
            "username": "Forum Bot",
            "avatar_url": "https://example.com/a.png"
        })))
        .respond_with(message("12", "99"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(format!("{HOOK_PATH}/messages/12")))
        .and(query_param("thread_id", "99"))
        .and(body_json(json!({ "content": "b" })))
        .respond_with(message("12", "99"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("{HOOK_PATH}/messages/12")))
        .and(query_param("thread_id", "99"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let msg = MessageBuilder::new()
        .content("a")
        .username("Forum Bot")
        .avatar_url("https://example.com/a.png");
    let sent = hook.send_to_thread(msg, 99).await.unwrap();
    assert_eq!((sent.message_id, sent.channel_id), (12, 99));
    hook.edit_thread_msg(EditMessage::new().content("b"), 99, 12)
        .await
        .unwrap();
    hook.delete_thread_msg(99, 12).await.unwrap();
}

// ---- retries and rate limits ----

#[tokio::test]
async fn rate_limit_without_retries_is_an_error() {
    let (server, hook) = setup().await;
    Mock::given(method("POST"))
        .respond_with(rate_limited(1.5))
        .mount(&server)
        .await;

    match hook.send_message(MessageBuilder::new().content("x")).await {
        Err(WebhookError::RateLimited {
            retry_after,
            global,
        }) => {
            assert_eq!(retry_after, 1.5);
            assert!(!global);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn rate_limit_is_retried_when_enabled() {
    let (server, hook) = setup().await;
    let hook = hook.with_max_retries(1);
    Mock::given(method("POST"))
        .respond_with(rate_limited(0.01))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(message("11", "20"))
        .expect(1)
        .mount(&server)
        .await;

    let sent = hook
        .send_message(MessageBuilder::new().content("x"))
        .await
        .unwrap();
    assert_eq!(sent.message_id, 11);
}

#[tokio::test]
async fn server_errors_are_retried_when_enabled() {
    let (server, hook) = setup().await;
    let hook = hook.with_max_retries(1);
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(message("14", "20"))
        .expect(1)
        .mount(&server)
        .await;

    let start = Instant::now();
    let sent = hook
        .send_message(MessageBuilder::new().content("x"))
        .await
        .unwrap();
    assert_eq!(sent.message_id, 14);
    assert!(
        start.elapsed() >= Duration::from_millis(450),
        "should back off"
    );
}

#[tokio::test]
async fn server_errors_are_not_retried_by_default() {
    let (server, hook) = setup().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(502))
        .expect(1)
        .mount(&server)
        .await;

    assert!(matches!(
        hook.send_message(MessageBuilder::new().content("x")).await,
        Err(WebhookError::Api { status: 502, .. })
    ));
}

#[tokio::test]
async fn rate_limit_is_shared_between_clones() {
    let (server, hook) = setup().await;
    Mock::given(method("POST"))
        .respond_with(rate_limited(5.0))
        .expect(1) // the clone must not send at all
        .mount(&server)
        .await;

    let other = hook.clone();
    assert!(
        hook.send_message(MessageBuilder::new().content("a"))
            .await
            .is_err()
    );
    match other.send_message(MessageBuilder::new().content("b")).await {
        Err(WebhookError::RateLimited { retry_after, .. }) => {
            assert!(retry_after > 4.0 && retry_after <= 5.0, "{retry_after}");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn empty_rate_limit_window_blocks_before_a_429() {
    let (server, hook) = setup().await;
    Mock::given(method("POST"))
        .respond_with(
            message("15", "20")
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("x-ratelimit-reset-after", "5"),
        )
        .expect(1)
        .mount(&server)
        .await;

    hook.send_message(MessageBuilder::new().content("a"))
        .await
        .unwrap();
    assert!(matches!(
        hook.send_message(MessageBuilder::new().content("b")).await,
        Err(WebhookError::RateLimited { .. })
    ));
}

// ---- webhook settings ----

#[tokio::test]
async fn modify_sets_name_and_avatar_as_data_uri() {
    let (server, hook) = setup().await;
    Mock::given(method("PATCH"))
        .and(path(HOOK_PATH))
        .and(body_json(json!({
            "name": "Deploy Bot",
            "avatar": "data:image/png;base64,iVBORw=="
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "1", "name": "Deploy Bot", "avatar": "abc123",
            "channel_id": "20", "guild_id": "30", "type": 1
        })))
        .expect(1)
        .mount(&server)
        .await;

    let changes = ModifyWebhook::new()
        .name("Deploy Bot")
        .avatar([0x89, b'P', b'N', b'G'])
        .unwrap();
    let info = hook.modify(changes).await.unwrap();
    assert_eq!(info.name.as_deref(), Some("Deploy Bot"));
    assert_eq!(info.avatar.as_deref(), Some("abc123"));
}

#[tokio::test]
async fn modify_can_remove_avatar() {
    let (server, hook) = setup().await;
    Mock::given(method("PATCH"))
        .and(path(HOOK_PATH))
        .and(body_json(json!({ "avatar": null })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "1", "name": "Bot", "avatar": null
        })))
        .expect(1)
        .mount(&server)
        .await;

    let info = hook
        .modify(ModifyWebhook::new().remove_avatar())
        .await
        .unwrap();
    assert_eq!(info.avatar, None);
}

#[test]
fn avatar_rejects_unknown_image_data() {
    assert!(matches!(
        ModifyWebhook::new().avatar(b"not an image"),
        Err(WebhookError::InvalidImage(_))
    ));
}

// ---- attachments ----

/// Matches a multipart request whose body contains every `needle`.
fn multipart_with(needles: &'static [&'static str]) -> impl Fn(&Request) -> bool {
    move |req: &Request| {
        let is_multipart = req
            .headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("multipart/form-data"));
        let body = String::from_utf8_lossy(&req.body);
        is_multipart && needles.iter().all(|n| body.contains(n))
    }
}

fn message_with_attachment() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "id": "16", "channel_id": "20",
        "attachments": [{
            "id": "500", "filename": "a.png", "size": 4,
            "url": "https://cdn.discordapp.com/attachments/20/500/a.png"
        }]
    }))
}

#[tokio::test]
async fn send_message_uploads_files_as_multipart() {
    let (server, hook) = setup().await;
    Mock::given(method("POST"))
        .and(path(HOOK_PATH))
        .and(query_param("wait", "true"))
        .and(multipart_with(&[
            "name=\"payload_json\"",
            r#""filename":"a.png""#,
            r#""description":"chart""#,
            "name=\"files[0]\"; filename=\"a.png\"",
            "Content-Type: image/png",
            "PNGDATA",
        ]))
        .respond_with(message_with_attachment())
        .expect(1)
        .mount(&server)
        .await;

    let msg = MessageBuilder::new()
        .content("report")
        .attachment(Attachment::new("a.png", "PNGDATA").description("chart"));
    let sent = hook.send_message(msg).await.unwrap();
    assert_eq!(sent.attachments.len(), 1);
    assert_eq!(sent.attachments[0].id, 500);
    assert_eq!(sent.attachments[0].filename, "a.png");
    assert!(sent.attachments[0].url.ends_with("/a.png"));
}

#[tokio::test]
async fn upload_is_rebuilt_for_a_retry() {
    let (server, hook) = setup().await;
    let hook = hook.with_max_retries(1);
    Mock::given(method("POST"))
        .and(multipart_with(&["FILEBODY"]))
        .respond_with(rate_limited(0.01))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(multipart_with(&["FILEBODY"]))
        .respond_with(message_with_attachment())
        .expect(1)
        .mount(&server)
        .await;

    let msg = MessageBuilder::new().attachment(Attachment::new("a.png", "FILEBODY"));
    hook.send_message(msg).await.unwrap();
}

#[tokio::test]
async fn edit_can_add_a_file_and_keep_an_old_one() {
    let (server, hook) = setup().await;
    Mock::given(method("PATCH"))
        .and(path(format!("{HOOK_PATH}/messages/16")))
        .and(multipart_with(&[
            r#"{"id":"500"}"#,
            r#""filename":"b.txt""#,
            "name=\"files[0]\"; filename=\"b.txt\"",
        ]))
        .respond_with(message_with_attachment())
        .expect(1)
        .mount(&server)
        .await;

    let edit = EditMessage::new()
        .attach(Attachment::new("b.txt", "log"))
        .keep_attachments([500]);
    hook.edit_message(edit, 16).await.unwrap();
}

#[tokio::test]
async fn forum_post_can_carry_files() {
    let (server, hook) = setup().await;
    Mock::given(method("POST"))
        .and(multipart_with(&[
            r#""thread_name":"Screens""#,
            "name=\"files[1]\"; filename=\"2.png\"",
        ]))
        .respond_with(message_with_attachment())
        .expect(1)
        .mount(&server)
        .await;

    let msg = MessageBuilder::new()
        .attachments([Attachment::new("1.png", "a"), Attachment::new("2.png", "b")]);
    hook.create_thread(ForumPost::new("Screens", msg))
        .await
        .unwrap();
}
