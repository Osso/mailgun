use anyhow::{Context, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::time::Duration;

use crate::config::Region;

const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_SECS: u64 = 1;
const EVENTS_MAX_LIMIT: u32 = 300;

pub struct Client {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    domain: String,
}

fn is_transient_error(err: &reqwest::Error) -> bool {
    let err_string = format!("{:?}", err);
    err_string.contains("os error 110")
        || err_string.contains("Connection timed out")
        || err_string.contains("connection reset")
        || err.is_timeout()
}

trait Paginated {
    fn paging(&self) -> Option<&Paging>;
    fn set_paging(&mut self, paging: Option<Paging>);
    fn extend_items(&mut self, other: Self);
    fn items_empty(other: &Self) -> bool;
}

impl Client {
    pub fn new(api_key: &str, domain: &str, region: Region) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        Ok(Self {
            http,
            base_url: region.base_url().to_string(),
            api_key: api_key.to_string(),
            domain: domain.to_string(),
        })
    }

    async fn send_with_retry<F, Fut>(
        &self,
        make_request: F,
    ) -> Result<reqwest::Response, reqwest::Error>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
    {
        let mut last_error = None;

        for attempt in 0..=MAX_RETRIES {
            match make_request().await {
                Ok(resp) => return Ok(resp),
                Err(err) if attempt < MAX_RETRIES && is_transient_error(&err) => {
                    let delay = INITIAL_BACKOFF_SECS * 2u64.pow(attempt);
                    eprintln!(
                        "Request failed, retrying ({}/{})...",
                        attempt + 1,
                        MAX_RETRIES
                    );
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    last_error = Some(err);
                }
                Err(err) => return Err(err),
            }
        }

        Err(last_error.expect("should have an error after retries"))
    }

    async fn request(&self, method: reqwest::Method, url: &str) -> Result<Value> {
        let url = url.to_string();

        let resp = self
            .send_with_retry(|| {
                self.http
                    .request(method.clone(), &url)
                    .basic_auth("api", Some(&self.api_key))
                    .send()
            })
            .await
            .context("Failed to send request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("HTTP {} - {}", status, body);
        }

        resp.json().await.context("Failed to parse JSON response")
    }

    async fn fetch_json(&self, url: &str) -> Result<Value> {
        self.request(reqwest::Method::GET, url).await
    }

    async fn get(&self, endpoint: &str) -> Result<Value> {
        let url = format!("{}{}", self.base_url, endpoint);
        self.fetch_json(&url).await
    }

    async fn delete(&self, endpoint: &str) -> Result<Value> {
        let url = format!("{}{}", self.base_url, endpoint);
        self.request(reqwest::Method::DELETE, &url).await
    }

    async fn paginated_list<T>(&self, endpoint: &str) -> Result<T>
    where
        T: DeserializeOwned + Paginated,
    {
        let value = self.get(endpoint).await?;
        let mut response: T = serde_json::from_value(value)?;

        loop {
            let next_url = response.paging().and_then(|p| p.next.clone());

            match next_url {
                None => break,
                Some(url) => {
                    let next_value = self.fetch_json(&url).await?;
                    let next_page: T = serde_json::from_value(next_value)?;

                    if T::items_empty(&next_page) {
                        break;
                    }

                    let new_paging = next_page.paging().cloned();
                    response.extend_items(next_page);
                    response.set_paging(new_paging);
                }
            }
        }

        Ok(response)
    }

    /// List events with optional filters
    pub async fn list_events(
        &self,
        event_type: Option<&str>,
        recipient: Option<&str>,
        limit: u32,
    ) -> Result<EventsResponse> {
        if limit > EVENTS_MAX_LIMIT {
            anyhow::bail!(
                "Limit exceeds maximum allowed by Mailgun API ({})",
                EVENTS_MAX_LIMIT
            );
        }

        let mut params = vec![format!("limit={}", limit)];

        if let Some(event) = event_type {
            params.push(format!("event={}", event));
        }
        if let Some(r) = recipient {
            params.push(format!("recipient={}", r));
        }

        let query = if params.is_empty() {
            String::new()
        } else {
            format!("?{}", params.join("&"))
        };

        let value = self
            .get(&format!("/{}/events{}", self.domain, query))
            .await?;

        Ok(serde_json::from_value(value)?)
    }

    /// List bounces (suppression list) - fetches all pages
    pub async fn list_bounces(&self, limit: u32) -> Result<BouncesResponse> {
        self.paginated_list(&format!("/{}/bounces?limit={}", self.domain, limit))
            .await
    }

    /// Delete a bounce (remove from suppression list)
    pub async fn delete_bounce(&self, email: &str) -> Result<Value> {
        self.delete(&format!("/{}/bounces/{}", self.domain, email))
            .await
    }

    /// List complaints (spam reports) - fetches all pages
    pub async fn list_complaints(&self, limit: u32) -> Result<ComplaintsResponse> {
        self.paginated_list(&format!("/{}/complaints?limit={}", self.domain, limit))
            .await
    }

    /// Delete a complaint (remove from complaints list)
    pub async fn delete_complaint(&self, email: &str) -> Result<Value> {
        self.delete(&format!("/{}/complaints/{}", self.domain, email))
            .await
    }

    /// List unsubscribes - fetches all pages
    pub async fn list_unsubscribes(&self, limit: u32) -> Result<UnsubscribesResponse> {
        self.paginated_list(&format!("/{}/unsubscribes?limit={}", self.domain, limit))
            .await
    }

    /// Delete an unsubscribe (remove from unsubscribe list)
    pub async fn delete_unsubscribe(&self, email: &str) -> Result<Value> {
        self.delete(&format!("/{}/unsubscribes/{}", self.domain, email))
            .await
    }

    /// Get sending statistics
    pub async fn get_stats(&self, event_types: &[&str], duration: &str) -> Result<StatsResponse> {
        let events = event_types.join(",");
        let value = self
            .get(&format!(
                "/{}/stats/total?event={}&duration={}",
                self.domain, events, duration
            ))
            .await?;
        Ok(serde_json::from_value(value)?)
    }

    /// Fetch a stored message by its storage URL
    pub async fn fetch_stored_message(&self, storage_url: &str) -> Result<StoredMessage> {
        let value = self.fetch_json(storage_url).await?;
        Ok(serde_json::from_value(value)?)
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EventsResponse {
    pub items: Vec<Event>,
    pub paging: Option<Paging>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Event {
    pub id: Option<String>,
    pub event: String,
    pub timestamp: f64,
    pub recipient: Option<String>,
    pub message: Option<MessageInfo>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(rename = "delivery-status")]
    pub delivery_status: Option<DeliveryStatus>,
    pub reason: Option<String>,
    pub severity: Option<String>,
    #[serde(rename = "storage")]
    pub storage: Option<StorageInfo>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MessageInfo {
    pub headers: Option<MessageHeaders>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MessageHeaders {
    #[serde(rename = "message-id")]
    pub message_id: Option<String>,
    pub subject: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DeliveryStatus {
    pub code: Option<i32>,
    pub message: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Paging {
    pub next: Option<String>,
    pub previous: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BouncesResponse {
    pub items: Vec<Bounce>,
    pub paging: Option<Paging>,
}

macro_rules! impl_paginated {
    ($type:ty) => {
        impl Paginated for $type {
            fn paging(&self) -> Option<&Paging> {
                self.paging.as_ref()
            }
            fn set_paging(&mut self, paging: Option<Paging>) {
                self.paging = paging;
            }
            fn extend_items(&mut self, other: Self) {
                self.items.extend(other.items);
            }
            fn items_empty(other: &Self) -> bool {
                other.items.is_empty()
            }
        }
    };
}

impl_paginated!(BouncesResponse);

#[derive(Debug, Deserialize, Serialize)]
pub struct Bounce {
    pub address: String,
    pub code: String,
    pub error: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ComplaintsResponse {
    pub items: Vec<Complaint>,
    pub paging: Option<Paging>,
}

impl_paginated!(ComplaintsResponse);

#[derive(Debug, Deserialize, Serialize)]
pub struct Complaint {
    pub address: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UnsubscribesResponse {
    pub items: Vec<Unsubscribe>,
    pub paging: Option<Paging>,
}

impl_paginated!(UnsubscribesResponse);

#[derive(Debug, Deserialize, Serialize)]
pub struct Unsubscribe {
    pub address: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StatsResponse {
    pub start: String,
    pub end: String,
    pub resolution: String,
    pub stats: Vec<StatEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StatEntry {
    pub time: String,
    pub accepted: Option<StatCount>,
    pub delivered: Option<StatCount>,
    pub failed: Option<StatCount>,
    pub opened: Option<StatCount>,
    pub clicked: Option<StatCount>,
    pub unsubscribed: Option<StatCount>,
    pub complained: Option<StatCount>,
    pub stored: Option<StatCount>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StatCount {
    pub total: Option<u64>,
    pub permanent: Option<u64>,
    pub temporary: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StorageInfo {
    pub url: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StoredMessage {
    /// Headers as array of [name, value] pairs from Mailgun's message-headers field
    #[serde(rename = "message-headers", deserialize_with = "deserialize_headers")]
    pub headers: StoredMessageHeaders,
    #[serde(rename = "From")]
    pub from: Option<String>,
    #[serde(rename = "To")]
    pub to: Option<String>,
    #[serde(rename = "Subject")]
    pub subject: Option<String>,
    #[serde(rename = "stripped-text")]
    pub stripped_text: Option<String>,
    #[serde(rename = "stripped-html")]
    pub stripped_html: Option<String>,
    #[serde(rename = "stripped-signature")]
    pub stripped_signature: Option<String>,
    #[serde(default)]
    pub attachments: Vec<AttachmentInfo>,
}

/// Deserialize message-headers from array of [name, value] pairs
fn deserialize_headers<'de, D>(deserializer: D) -> Result<StoredMessageHeaders, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let pairs: Vec<(String, String)> = Vec::deserialize(deserializer)?;
    let mut headers = StoredMessageHeaders::default();

    for (name, value) in pairs {
        match name.to_lowercase().as_str() {
            "received" => headers.received.push(value),
            "dkim-signature" => headers.dkim = Some(value),
            "mime-version" => headers.mime_version = Some(value),
            "content-transfer-encoding" => headers.content_transfer_encoding = Some(value),
            "list-unsubscribe" => headers.list_unsubscribe = Some(value),
            "list-unsubscribe-post" => headers.list_unsubscribe_post = Some(value),
            _ => {
                headers.other.push((name, value));
            }
        }
    }

    Ok(headers)
}

#[derive(Debug, Default, Serialize)]
pub struct StoredMessageHeaders {
    #[serde(default)]
    pub received: Vec<String>,
    pub dkim: Option<String>,
    pub mime_version: Option<String>,
    pub content_transfer_encoding: Option<String>,
    pub list_unsubscribe: Option<String>,
    pub list_unsubscribe_post: Option<String>,
    /// Other headers not explicitly parsed
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub other: Vec<(String, String)>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AttachmentInfo {
    pub filename: String,
    pub size: u64,
    #[serde(rename = "content-type")]
    pub content_type: String,
}
