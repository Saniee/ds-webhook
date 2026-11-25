//! **Discontinued:** this blocking version is no longer maintained.
//! Use the async (tokio) version: <https://github.com/Saniee/ds-webhook/tree/tokio>

use anyhow::{Ok, Result};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct WebhookWaitResp {
    pub id: Option<String>,
    #[serde(rename = "channel_id")]
    pub thread_id: Option<String>,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct MessageBuilder {
    #[serde(rename = "content")]
    pub message: Option<String>,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub embeds: Option<Vec<Embed>>,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
/** This `can be used to send messages to a thread, or create one`. If a thread_id is specified, a message will be sent into the specified thread. If a thread_name is specified, a new thread with that name will be created. */
pub struct ThreadBuilder {
    pub thread_name: Option<String>,
    #[serde(rename = "content")]
    pub thread_message: Option<String>,
    #[serde(rename = "embeds")]
    pub thread_embeds: Option<Vec<Embed>>,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Embed {
    pub title: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub timestamp: Option<u128>,
    pub color: Option<u64>,
    pub image: Option<Image>,
    pub fields: Option<Vec<Field>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub value: String,
    pub inline: bool,
}

pub struct DiscordWebhook {
    pub webhook_url: String,
    pub thread_webhook_url: String,
    pub req_client: reqwest::blocking::Client,
}

#[derive(Error, Debug)]
pub enum WebhookError {
    #[error("Invalid Webhook url! ({0})")]
    InvalidWebhook(String),
    #[error("Reqwest has thrown an exception: {0}!")]
    RequestError(reqwest::Error),
    #[error("Webhook API has thrown an exception: ErrorMsg > {0}!")]
    ApiError(String),
}

impl From<reqwest::Error> for WebhookError {
    fn from(value: reqwest::Error) -> Self {
        WebhookError::RequestError(value)
    }
}

async fn test_webhook_valid(url: &str) -> Result<reqwest::blocking::Client> {
    let client = reqwest::blocking::Client::new();

    let response = client.clone().get(url).send()?.error_for_status()?;

    if response.status() != StatusCode::OK {
        Err(WebhookError::InvalidWebhook(url.to_owned()).into())
    } else {
        Ok(client)
    }
}

impl DiscordWebhook {
    pub async fn new(url: &str, thread_url: &str) -> Result<Self> {
        let client = test_webhook_valid(url).await?;

        Ok(Self {
            webhook_url: url.to_owned(),
            thread_webhook_url: thread_url.to_owned(),
            req_client: client,
        })
    }

    pub fn hex_to_dec(&self, hex: &str) -> u64 {
        u64::from_str_radix(hex, 16).expect("Err")
    }

    /** Returns both the message id, and thread id. <br/> Message ID is first, Thread ID is second. */
    pub async fn create_thread(&self, thread: ThreadBuilder) -> Result<(u64, u64)> {
        let response = self
            .req_client
            .clone()
            .post(self.thread_webhook_url.clone() + "?wait=true")
            .json(&thread)
            .send()?
            .json::<WebhookWaitResp>()?;

        Ok((
            response.id.unwrap_or_default().parse::<u64>()?,
            response.thread_id.unwrap_or_default().parse::<u64>()?,
        ))
    }

    /** Returns both the message id, and thread id. <br/> Message ID is first, Thread ID is second. */
    pub async fn send_to_thread(
        &self,
        thread: ThreadBuilder,
        thread_id: u64,
    ) -> Result<(u64, u64)> {
        let response = self
            .req_client
            .post(self.thread_webhook_url.clone() + &format!("?wait=true&thread_id={thread_id}"))
            .json(&thread)
            .send()?
            .json::<WebhookWaitResp>()?;

        Ok((
            response
                .id
                .unwrap_or_default()
                .parse::<u64>()
                .unwrap_or_default(),
            response
                .thread_id
                .unwrap_or_default()
                .parse::<u64>()
                .unwrap_or_default(),
        ))
    }

    pub async fn edit_thread_msg(
        &self,
        thread: ThreadBuilder,
        thread_id: u64,
        msg_id: u64,
    ) -> Result<(u64, u64)> {
        let response = self
            .req_client
            .clone()
            .patch(
                self.thread_webhook_url.clone()
                    + &format!("/messages/{msg_id}?thread_id={thread_id}"),
            )
            .json(&thread)
            .send()?
            .json::<WebhookWaitResp>()?;

        Ok((
            response
                .id
                .unwrap_or_default()
                .parse::<u64>()
                .unwrap_or_default(),
            response
                .thread_id
                .unwrap_or_default()
                .parse::<u64>()
                .unwrap_or_default(),
        ))
    }

    pub async fn send_message(&self, content: MessageBuilder) -> Result<u64> {
        let response = self
            .req_client
            .clone()
            .post(self.webhook_url.clone() + "?wait=true")
            .json(&content)
            .send()?
            .json::<WebhookWaitResp>()?;

        Ok(response
            .id
            .unwrap_or_default()
            .parse::<u64>()
            .unwrap_or_default())
    }

    pub async fn edit_message(&self, content: MessageBuilder, msg_id: u64) -> Result<u64> {
        let response = self
            .req_client
            .clone()
            .patch(self.webhook_url.clone() + &format!("/messages/{msg_id}"))
            .json(&content)
            .send()?
            .json::<WebhookWaitResp>()?;

        Ok(response
            .id
            .unwrap_or_default()
            .parse::<u64>()
            .unwrap_or_default())
    }

    pub async fn delete_message(&self, msg_id: u64) -> Result<()> {
        let response = self
            .req_client
            .clone()
            .delete(self.webhook_url.clone() + &format!("/messages/{msg_id}"))
            .send()?;

        if let Err(msg) = response.error_for_status() {
            Err(WebhookError::ApiError(msg.to_string()).into())
        } else {
            Ok(())
        }
    }
}
