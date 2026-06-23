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
    fn paging(&self) -> Option<&Paging> {
        None
    }

    fn set_paging(&mut self, _paging: Option<Paging>) {}

    fn extend_items(&mut self, _other: Self)
    where
        Self: Sized,
    {
    }

    fn items_empty(_other: &Self) -> bool {
        true
    }
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

    #[cfg(test)]
    pub(crate) fn with_base_url(api_key: &str, domain: &str, base_url: String) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        Ok(Self {
            http,
            base_url,
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

    /// GET against the v4 API (the domains endpoint lives on v4, not v3).
    async fn get_v4(&self, endpoint: &str) -> Result<Value> {
        let url = format!("{}{}", self.base_url.replace("/v3", "/v4"), endpoint);
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

    /// List all IP addresses on the account, optionally only dedicated ones
    pub async fn list_account_ips(&self, dedicated: bool) -> Result<IpsResponse> {
        let query = if dedicated { "?dedicated=true" } else { "" };
        let value = self.get(&format!("/ips{}", query)).await?;
        Ok(serde_json::from_value(value)?)
    }

    /// List IP addresses assigned to a domain (defaults to the configured one)
    pub async fn list_domain_ips(&self, domain: Option<&str>) -> Result<IpsResponse> {
        let domain = domain.unwrap_or(&self.domain);
        let value = self.get(&format!("/domains/{}/ips", domain)).await?;
        Ok(serde_json::from_value(value)?)
    }

    /// List all domains on the account
    pub async fn list_domains(&self, limit: u32) -> Result<DomainsResponse> {
        let value = self.get_v4(&format!("/domains?limit={}", limit)).await?;
        Ok(serde_json::from_value(value)?)
    }

    /// Get details for a single IP address
    pub async fn get_ip(&self, ip: &str) -> Result<IpDetails> {
        let value = self.get(&format!("/ips/{}", ip)).await?;
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
pub struct IpsResponse {
    // Mailgun returns `null` (not an empty array) when there are no items,
    // so accept both null and missing as an empty list.
    #[serde(default, deserialize_with = "deserialize_null_vec")]
    pub items: Vec<String>,
    pub total_count: Option<u32>,
}

fn deserialize_null_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<Vec<String>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DomainsResponse {
    #[serde(default)]
    pub items: Vec<Domain>,
    pub total_count: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Domain {
    pub name: String,
    pub state: Option<String>,
    #[serde(rename = "type")]
    pub domain_type: Option<String>,
    #[serde(default)]
    pub is_disabled: bool,
    pub created_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct IpDetails {
    pub ip: String,
    pub rdns: Option<String>,
    pub dedicated: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AttachmentInfo {
    pub filename: String,
    pub size: u64,
    #[serde(rename = "content-type")]
    pub content_type: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    struct MockServer {
        base_url: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl MockServer {
        async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let server_requests = Arc::clone(&requests);

            tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        break;
                    };
                    let requests = Arc::clone(&server_requests);
                    tokio::spawn(async move {
                        let mut buffer = vec![0; 4096];
                        let bytes_read = stream.read(&mut buffer).await.unwrap();
                        let request = String::from_utf8_lossy(&buffer[..bytes_read]);
                        let request_line = request.lines().next().unwrap_or_default().to_string();
                        let body = response_body(&request_line);
                        requests.lock().unwrap().push(request_line);
                        let response = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        stream.write_all(response.as_bytes()).await.unwrap();
                    });
                }
            });

            Self {
                base_url: format!("http://{}/v3", display_addr(addr)),
                requests,
            }
        }

        fn client(&self) -> Client {
            Client::with_base_url("key-test", "example.com", self.base_url.clone()).unwrap()
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    fn display_addr(addr: SocketAddr) -> String {
        format!("{}:{}", addr.ip(), addr.port())
    }

    fn response_body(request_line: &str) -> &'static str {
        let path = request_line.split_whitespace().nth(1).unwrap_or_default();
        response_for_path(path).unwrap_or(r#"{}"#)
    }

    fn response_for_path(path: &str) -> Option<&'static str> {
        suppression_response(path)
            .or_else(|| stats_response(path))
            .or_else(|| message_response(path))
            .or_else(|| ip_response(path))
            .or_else(|| domain_response(path))
    }

    fn suppression_response(path: &str) -> Option<&'static str> {
        match path {
            "/v3/example.com/events?limit=2&event=failed&recipient=reader@example.com" => Some(
                r#"{"items":[{"event":"failed","timestamp":1710000000,"recipient":"reader@example.com"}],"paging":{"next":null,"previous":null}}"#,
            ),
            "/v3/example.com/bounces?limit=1" => Some(
                r#"{"items":[{"address":"bad@example.com","code":"550","error":"blocked","created_at":"2026-01-01"}],"paging":null}"#,
            ),
            "/v3/example.com/complaints?limit=1" => Some(
                r#"{"items":[{"address":"spam@example.com","created_at":"2026-01-02"}],"paging":null}"#,
            ),
            "/v3/example.com/unsubscribes?limit=1" => Some(
                r#"{"items":[{"address":"gone@example.com","tags":["news"],"created_at":"2026-01-03"}],"paging":null}"#,
            ),
            "/v3/example.com/bounces/bad@example.com"
            | "/v3/example.com/complaints/spam@example.com"
            | "/v3/example.com/unsubscribes/gone@example.com" => Some(r#"{"deleted":true}"#),
            _ => None,
        }
    }

    fn stats_response(path: &str) -> Option<&'static str> {
        match path {
            "/v3/example.com/stats/total?event=accepted,delivered&duration=7d" => Some(
                r#"{"start":"2026-01-01","end":"2026-01-08","resolution":"day","stats":[{"time":"2026-01-01","accepted":{"total":3},"delivered":{"total":2}}]}"#,
            ),
            _ => None,
        }
    }

    fn message_response(path: &str) -> Option<&'static str> {
        match path {
            "/v3/message" => Some(
                r#"{"message-headers":[["Received","mx1"]],"From":"sender@example.com","To":"reader@example.com","Subject":"Stored","stripped-text":"Body","attachments":[]}"#,
            ),
            _ => None,
        }
    }

    fn ip_response(path: &str) -> Option<&'static str> {
        match path {
            "/v3/ips" => Some(r#"{"items":["1.2.3.4"],"total_count":1}"#),
            "/v3/ips?dedicated=true" => Some(r#"{"items":["5.6.7.8"],"total_count":1}"#),
            "/v3/domains/example.com/ips" => Some(r#"{"items":["9.9.9.9"],"total_count":1}"#),
            "/v3/domains/other.com/ips" => Some(r#"{"items":["8.8.8.8"],"total_count":1}"#),
            "/v3/ips/1.2.3.4" => {
                Some(r#"{"ip":"1.2.3.4","rdns":"mail.example.com","dedicated":true}"#)
            }
            _ => None,
        }
    }

    fn domain_response(path: &str) -> Option<&'static str> {
        match path {
            "/v4/domains?limit=5" => Some(
                r#"{"items":[{"name":"example.com","state":"active","type":"sandbox","is_disabled":false,"created_at":"2026-01-01"}],"total_count":1}"#,
            ),
            _ => None,
        }
    }

    #[test]
    fn rejects_event_limits_above_mailgun_maximum_before_requesting() {
        let client = Client::new("key-test", "example.com", Region::Us).unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let err = runtime
            .block_on(client.list_events(None, None, EVENTS_MAX_LIMIT + 1))
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("Limit exceeds maximum allowed by Mailgun API (300)")
        );
    }

    #[test]
    fn parses_stored_message_headers_into_known_and_other_groups() {
        let message: StoredMessage = serde_json::from_value(json!({
            "message-headers": [
                ["Received", "mx1"],
                ["DKIM-Signature", "dkim"],
                ["Mime-Version", "1.0"],
                ["Content-Transfer-Encoding", "quoted-printable"],
                ["List-Unsubscribe", "<mailto:unsubscribe@example.com>"],
                ["List-Unsubscribe-Post", "List-Unsubscribe=One-Click"],
                ["X-Campaign", "summer"]
            ],
            "From": "sender@example.com",
            "To": "reader@example.com",
            "Subject": "Hello",
            "stripped-text": "Body",
            "stripped-html": "<p>Body</p>",
            "stripped-signature": "Sig",
            "attachments": [{
                "filename": "report.pdf",
                "size": 1024,
                "content-type": "application/pdf"
            }]
        }))
        .unwrap();

        assert_eq!(message.headers.received, vec!["mx1"]);
        assert_eq!(message.headers.dkim.as_deref(), Some("dkim"));
        assert_eq!(message.headers.mime_version.as_deref(), Some("1.0"));
        assert_eq!(
            message.headers.content_transfer_encoding.as_deref(),
            Some("quoted-printable")
        );
        assert_eq!(
            message.headers.list_unsubscribe.as_deref(),
            Some("<mailto:unsubscribe@example.com>")
        );
        assert_eq!(
            message.headers.list_unsubscribe_post.as_deref(),
            Some("List-Unsubscribe=One-Click")
        );
        assert_eq!(
            message.headers.other,
            vec![("X-Campaign".to_string(), "summer".to_string())]
        );
        assert_eq!(message.attachments[0].filename, "report.pdf");
    }

    #[test]
    fn accepts_null_or_missing_ip_items_as_empty_lists() {
        let null_items: IpsResponse =
            serde_json::from_value(json!({"items": null, "total_count": 0})).unwrap();
        let missing_items: IpsResponse = serde_json::from_value(json!({})).unwrap();

        assert!(null_items.items.is_empty());
        assert_eq!(null_items.total_count, Some(0));
        assert!(missing_items.items.is_empty());
        assert_eq!(missing_items.total_count, None);
    }

    #[test]
    fn defaults_optional_collections_when_deserializing_api_records() {
        let event: Event = serde_json::from_value(json!({
            "event": "delivered",
            "timestamp": 1710000000.0,
            "delivery-status": {"code": 250, "message": "OK"}
        }))
        .unwrap();
        let unsubscribe: Unsubscribe = serde_json::from_value(json!({
            "address": "reader@example.com",
            "created_at": "2026-01-01"
        }))
        .unwrap();
        let domain: Domain = serde_json::from_value(json!({
            "name": "example.com"
        }))
        .unwrap();

        assert!(event.tags.is_empty());
        assert_eq!(event.delivery_status.unwrap().code, Some(250));
        assert!(unsubscribe.tags.is_empty());
        assert!(!domain.is_disabled);
    }

    #[test]
    fn paginated_response_helpers_merge_items_and_replace_paging() {
        let mut response = BouncesResponse {
            items: vec![Bounce {
                address: "a@example.com".to_string(),
                code: "550".to_string(),
                error: "blocked".to_string(),
                created_at: "2026-01-01".to_string(),
            }],
            paging: Some(Paging {
                next: Some("https://next".to_string()),
                previous: None,
            }),
        };
        let next_page = BouncesResponse {
            items: vec![Bounce {
                address: "b@example.com".to_string(),
                code: "551".to_string(),
                error: "invalid".to_string(),
                created_at: "2026-01-02".to_string(),
            }],
            paging: Some(Paging {
                next: None,
                previous: Some("https://previous".to_string()),
            }),
        };

        assert_eq!(
            response.paging().and_then(|paging| paging.next.as_deref()),
            Some("https://next")
        );
        assert!(!BouncesResponse::items_empty(&next_page));

        let new_paging = next_page.paging().cloned();
        response.extend_items(next_page);
        response.set_paging(new_paging);

        assert_eq!(response.items.len(), 2);
        assert_eq!(
            response
                .paging()
                .and_then(|paging| paging.previous.as_deref()),
            Some("https://previous")
        );
    }

    #[test]
    fn paginated_helpers_work_for_suppression_response_types() {
        let complaints = ComplaintsResponse {
            items: Vec::new(),
            paging: None,
        };
        let unsubscribes = UnsubscribesResponse {
            items: Vec::new(),
            paging: None,
        };

        assert!(ComplaintsResponse::items_empty(&complaints));
        assert!(UnsubscribesResponse::items_empty(&unsubscribes));
    }

    #[tokio::test]
    async fn client_fetches_events_and_suppression_lists_from_expected_paths() {
        let server = MockServer::start().await;
        let client = server.client();

        let events = client
            .list_events(Some("failed"), Some("reader@example.com"), 2)
            .await
            .unwrap();
        let bounces = client.list_bounces(1).await.unwrap();
        let complaints = client.list_complaints(1).await.unwrap();
        let unsubscribes = client.list_unsubscribes(1).await.unwrap();

        assert_eq!(events.items.len(), 1);
        assert_eq!(events.items[0].event, "failed");
        assert_eq!(bounces.items[0].address, "bad@example.com");
        assert_eq!(complaints.items[0].address, "spam@example.com");
        assert_eq!(unsubscribes.items[0].tags, vec!["news"]);
        assert!(server
            .requests()
            .contains(&"GET /v3/example.com/events?limit=2&event=failed&recipient=reader@example.com HTTP/1.1".to_string()));
    }

    #[tokio::test]
    async fn client_deletes_suppression_entries() {
        let server = MockServer::start().await;
        let client = server.client();

        assert_eq!(
            client.delete_bounce("bad@example.com").await.unwrap()["deleted"],
            true
        );
        assert_eq!(
            client.delete_complaint("spam@example.com").await.unwrap()["deleted"],
            true
        );
        assert_eq!(
            client.delete_unsubscribe("gone@example.com").await.unwrap()["deleted"],
            true
        );

        let requests = server.requests();
        assert!(
            requests
                .contains(&"DELETE /v3/example.com/bounces/bad@example.com HTTP/1.1".to_string())
        );
        assert!(
            requests.contains(
                &"DELETE /v3/example.com/complaints/spam@example.com HTTP/1.1".to_string()
            )
        );
        assert!(requests.contains(
            &"DELETE /v3/example.com/unsubscribes/gone@example.com HTTP/1.1".to_string()
        ));
    }

    #[tokio::test]
    async fn client_fetches_stats_stored_messages_ips_and_domains() {
        let server = MockServer::start().await;
        let client = server.client();

        let stats = client
            .get_stats(&["accepted", "delivered"], "7d")
            .await
            .unwrap();
        let stored = client
            .fetch_stored_message(&format!("{}/message", server.base_url))
            .await
            .unwrap();
        let account_ips = client.list_account_ips(false).await.unwrap();
        let dedicated_ips = client.list_account_ips(true).await.unwrap();
        let domain_ips = client.list_domain_ips(None).await.unwrap();
        let other_domain_ips = client.list_domain_ips(Some("other.com")).await.unwrap();
        let ip = client.get_ip("1.2.3.4").await.unwrap();
        let domains = client.list_domains(5).await.unwrap();

        assert_eq!(stats.stats[0].accepted.as_ref().unwrap().total, Some(3));
        assert_eq!(stored.subject.as_deref(), Some("Stored"));
        assert_eq!(account_ips.items, vec!["1.2.3.4"]);
        assert_eq!(dedicated_ips.items, vec!["5.6.7.8"]);
        assert_eq!(domain_ips.items, vec!["9.9.9.9"]);
        assert_eq!(other_domain_ips.items, vec!["8.8.8.8"]);
        assert_eq!(ip.rdns.as_deref(), Some("mail.example.com"));
        assert_eq!(domains.items[0].name, "example.com");
    }
}
