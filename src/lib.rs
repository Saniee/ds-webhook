use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use anyhow::Result;
use thiserror::Error;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct WebhookWaitResp {
    pub id: Option<String>,
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
pub struct Embed {
    pub title: Option<String>,
    pub description: String,
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
}

pub struct DiscordWebhook {
    pub webhook_url: String,
    pub req_client: reqwest::Client,
}

#[derive(Error, Debug)]
pub enum WebhookError {
    #[error("Invalid webhook with url: {0}!")]
    InvalidWebhook(String),
    #[error("Reqwest thrown an exception: {0}")]
    RequestError(reqwest::Error)
}

impl From<reqwest::Error> for WebhookError {
    fn from(value: reqwest::Error) -> Self {
        WebhookError::RequestError(value)
    }
}

async fn test_webhook_valid(url: &str) -> Result<reqwest::Client> {
    let client = reqwest::Client::new();
    
    let response = client
        .clone()
        .get(url)
        .send()
        .await?.error_for_status()?;

    if response.status() != StatusCode::OK {
        Err(WebhookError::InvalidWebhook(url.to_owned()).into())
    } else {
        Ok(client)
    }
}

impl DiscordWebhook {
    pub async fn new(url: &str) -> Result<Self> {
        let client = test_webhook_valid(url).await?;

        Ok(Self {
            webhook_url: url.to_owned(),
            req_client: client
        })
    }

    pub fn hex_to_dec(&self, hex: &str) -> u64 {
        u64::from_str_radix(hex, 16).expect("Err")
    }

    pub async fn send(&self, content: MessageBuilder) -> u64 {
        let response = self
            .req_client
            .clone()
            .post(self.webhook_url.clone() + "?wait=true")
            .json(&content)
            .send()
            .await
            .expect("Err")
            .json::<WebhookWaitResp>()
            .await;
            // .expect("Err");
        
        match response {
            Ok(response) => {
                response
                    .id
                    .unwrap_or_default()
                    .parse::<u64>()
                    .unwrap_or_default()
            },
            Err(msg) => {
                println!("An error occured at sending a webhook msg: {}", msg);
                0
            }
        }
    }

    pub async fn edit(&self, content: MessageBuilder, msg_id: u64) -> u64 {
        let response = self
            .req_client
            .clone()
            .patch(self.webhook_url.clone() + &format!("/messages/{}", msg_id))
            .json(&content)
            .send()
            .await
            .expect("Err")
            .json::<WebhookWaitResp>()
            .await;
            // .expect("Err");
        
            match response {
                Ok(response) => {
                    response
                        .id
                        .unwrap_or_default()
                        .parse::<u64>()
                        .unwrap_or_default()
                },
                Err(msg) => {
                    println!("An error occured at editing a webhook msg: {}", msg);
                    0
                }
            }
    }

    pub async fn delete(&self, msg_id: u64) {
        let response = self.req_client
            .clone()
            .delete(self.webhook_url.clone() + &format!("/messages/{}", msg_id))
            .send()
            .await;
            // .expect("Err");
        
        match response {
            Ok(_) => {
                println!("Deleted webhook msg with id: {}", msg_id);
            },
            Err(msg) => {
                println!("An error occured at deleting a webhook msg: {}", msg);
            }
        }
    }
}
