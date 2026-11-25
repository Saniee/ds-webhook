use ds_webhook::{DiscordWebhook, MessageBuilder, ThreadBuilder, WebhookError};
use serde_json::json;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const HOOK_PATH: &str = "/api/webhooks/1/token";

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
    assert_eq!((sent.message_id, sent.thread_id), (10, 20));
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
async fn rate_limit_without_retries_is_an_error() {
    let (server, hook) = setup().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).set_body_json(
            json!({ "message": "You are being rate limited.", "retry_after": 1.5, "global": false }),
        ))
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
        .respond_with(ResponseTemplate::new(429).set_body_json(
            json!({ "message": "rate limited", "retry_after": 0.01, "global": false }),
        ))
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
async fn thread_calls_use_thread_id_and_message_path() {
    let (server, hook) = setup().await;
    Mock::given(method("POST"))
        .and(path(HOOK_PATH))
        .and(query_param("wait", "true"))
        .and(query_param("thread_id", "99"))
        .respond_with(message("12", "99"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(format!("{HOOK_PATH}/messages/12")))
        .and(query_param("thread_id", "99"))
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

    let sent = hook
        .send_to_thread(ThreadBuilder::new().content("a"), 99)
        .await
        .unwrap();
    assert_eq!((sent.message_id, sent.thread_id), (12, 99));
    hook.edit_thread_msg(ThreadBuilder::new().content("b"), 99, 12)
        .await
        .unwrap();
    hook.delete_thread_msg(99, 12).await.unwrap();
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
