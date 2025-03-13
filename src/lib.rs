use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct WebhookWaitResp {
    pub id: Option<String>,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct WebhookContent {
    #[serde(rename = "content")]
    pub message: Option<String>,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub embeds: Option<Vec<Embed>>,
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
}

pub struct DiscordWebhook {
    pub webhook_url: String,
    pub req_client: reqwest::Client,
}

impl DiscordWebhook {
    pub async fn test_webhook_valid(&self) -> bool {
        let response = self
            .req_client
            .clone()
            .get(&self.webhook_url)
            .send()
            .await;
            // .expect("Err");
        
        match response {
            Ok(response) => {
                response.status() == 200
            },
            Err(msg) => {
                println!("An error occured at testing the webhook validity: {}", msg);
                false
            }
        }
    }

    pub fn hex_to_dec(&self, hex: &str) -> u64 {
        u64::from_str_radix(hex, 16).expect("Err")
    }

    pub async fn send(&self, content: WebhookContent) -> u64 {
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

    pub async fn edit(&self, content: WebhookContent, msg_id: u64) -> u64 {
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
