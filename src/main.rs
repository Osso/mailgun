mod api;
mod config;

use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};

use config::Region;

#[derive(Parser)]
#[command(name = "mailgun")]
#[command(about = "CLI tool to access Mailgun API")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List recent events (accepted, delivered, failed, etc.)
    Events {
        /// Filter by event type (accepted, delivered, failed, rejected, clicked, opened, etc.)
        #[arg(short, long)]
        event: Option<String>,
        /// Filter by recipient email
        #[arg(short, long)]
        recipient: Option<String>,
        /// Number of results to return (max 300)
        #[arg(short = 'n', long, default_value = "20")]
        limit: u32,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Fetch a stored message by its storage URL
    Message {
        /// Storage URL from event data
        storage_url: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Show only headers
        #[arg(long)]
        headers: bool,
    },
    /// List suppressed/bounced addresses
    Bounces {
        /// Number of results to return
        #[arg(short = 'n', long, default_value = "20")]
        limit: u32,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<BouncesCommands>,
    },
    /// List complaint addresses (spam reports)
    Complaints {
        /// Number of results to return
        #[arg(short = 'n', long, default_value = "20")]
        limit: u32,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<ComplaintsCommands>,
    },
    /// List unsubscribed addresses
    Unsubscribes {
        /// Number of results to return
        #[arg(short = 'n', long, default_value = "20")]
        limit: u32,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<UnsubscribesCommands>,
    },
    /// Show sending statistics
    Stats {
        /// Duration to show stats for (1h, 24h, 7d, 30d)
        #[arg(short, long, default_value = "7d")]
        duration: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Configure API key and domain
    Config {
        /// Mailgun API key
        #[arg(short = 'k', long)]
        api_key: Option<String>,
        /// Mailgun domain (e.g., mg.example.com)
        #[arg(short, long)]
        domain: Option<String>,
        /// Region: us or eu
        #[arg(short, long)]
        region: Option<String>,
    },
}

#[derive(Subcommand)]
enum BouncesCommands {
    /// Remove an address from the bounce list
    Delete {
        /// Email address to remove
        email: String,
    },
}

#[derive(Subcommand)]
enum ComplaintsCommands {
    /// Remove an address from the complaints list
    Delete {
        /// Email address to remove
        email: String,
    },
}

#[derive(Subcommand)]
enum UnsubscribesCommands {
    /// Remove an address from the unsubscribes list
    Delete {
        /// Email address to remove
        email: String,
    },
}

fn get_client() -> Result<api::Client> {
    let cfg = config::load_config()?;

    let api_key = cfg
        .api_key
        .ok_or_else(|| anyhow::anyhow!("API key not configured. Run 'mailgun config -k <key>'"))?;

    let domain = cfg.domain.ok_or_else(|| {
        anyhow::anyhow!("Domain not configured. Run 'mailgun config -d <domain>'")
    })?;

    api::Client::new(&api_key, &domain, cfg.region)
}

fn format_timestamp(ts: f64) -> String {
    DateTime::<Utc>::from_timestamp(ts as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| ts.to_string())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let truncated: String = s.chars().take(max).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}

fn format_delivery_status(status: &api::DeliveryStatus) -> String {
    match (status.code, status.message.as_deref()) {
        (Some(code), Some(msg)) => format!(" [{}] {}", code, truncate(msg, 50)),
        (Some(code), None) => format!(" [{}]", code),
        (None, Some(msg)) => format!(" {}", truncate(msg, 50)),
        (None, None) => String::new(),
    }
}

fn print_events(resp: api::EventsResponse) {
    if resp.items.is_empty() {
        println!("No events found.");
        return;
    }

    for event in resp.items {
        let time = format_timestamp(event.timestamp);
        let recipient = event.recipient.as_deref().unwrap_or("-");
        let subject = event
            .message
            .as_ref()
            .and_then(|m| m.headers.as_ref())
            .and_then(|h| h.subject.as_deref())
            .unwrap_or("-");

        print!("{} {:10} {}", time, event.event, recipient);

        if let Some(status) = &event.delivery_status {
            print!("{}", format_delivery_status(status));
        } else if let Some(reason) = &event.reason {
            print!(" ({})", reason);
        }

        if subject != "-" {
            print!(" \"{}\"", truncate(subject, 40));
        }

        println!();
    }
}

fn print_bounces(resp: api::BouncesResponse) {
    if resp.items.is_empty() {
        println!("No bounced addresses found.");
        return;
    }

    println!(
        "{:<40} {:>6} {:<30} {}",
        "ADDRESS", "CODE", "ERROR", "CREATED"
    );
    println!("{}", "-".repeat(100));

    for bounce in resp.items {
        println!(
            "{:<40} {:>6} {:<30} {}",
            bounce.address,
            bounce.code,
            truncate(&bounce.error, 30),
            bounce.created_at
        );
    }
}

fn print_complaints(resp: api::ComplaintsResponse) {
    if resp.items.is_empty() {
        println!("No complaint addresses found.");
        return;
    }

    println!("{:<50} {}", "ADDRESS", "CREATED");
    println!("{}", "-".repeat(80));

    for complaint in resp.items {
        println!("{:<50} {}", complaint.address, complaint.created_at);
    }
}

fn print_unsubscribes(resp: api::UnsubscribesResponse) {
    if resp.items.is_empty() {
        println!("No unsubscribed addresses found.");
        return;
    }

    println!("{:<40} {:<20} {}", "ADDRESS", "TAGS", "CREATED");
    println!("{}", "-".repeat(90));

    for unsub in resp.items {
        let tags = if unsub.tags.is_empty() {
            "-".to_string()
        } else {
            unsub.tags.join(", ")
        };
        println!(
            "{:<40} {:<20} {}",
            unsub.address,
            truncate(&tags, 20),
            unsub.created_at
        );
    }
}

fn print_headers(headers: &api::StoredMessageHeaders) {
    if !headers.received.is_empty() {
        for (i, received) in headers.received.iter().enumerate() {
            if i == 0 {
                println!("Received: {}", received);
            } else {
                println!("Received[{}]: {}", i, received);
            }
        }
    }
    if let Some(dkim) = &headers.dkim {
        println!("DKIM-Signature: {}", dkim);
    }
    if let Some(mime) = &headers.mime_version {
        println!("MIME-Version: {}", mime);
    }
    if let Some(cte) = &headers.content_transfer_encoding {
        println!("Content-Transfer-Encoding: {}", cte);
    }
    if let Some(unsub) = &headers.list_unsubscribe {
        println!("List-Unsubscribe: {}", unsub);
    }
    if let Some(unsub_post) = &headers.list_unsubscribe_post {
        println!("List-Unsubscribe-Post: {}", unsub_post);
    }
    for (name, value) in &headers.other {
        println!("{}: {}", name, value);
    }
}

fn print_message(msg: &api::StoredMessage, headers_only: bool) {
    if headers_only {
        println!("=== Headers ===");
        print_headers(&msg.headers);
        return;
    }

    println!("=== Message Details ===");
    if let Some(subject) = &msg.subject {
        println!("Subject: {}", subject);
    }
    if let Some(from) = &msg.from {
        println!("From: {}", from);
    }
    if let Some(to) = &msg.to {
        println!("To: {}", to);
    }

    println!();
    println!("=== Headers ===");
    print_headers(&msg.headers);

    println!();
    println!("=== Body ===");
    if let Some(text) = &msg.stripped_text {
        let preview: String = text.chars().take(500).collect();
        if text.len() > 500 {
            println!(
                "Text (truncated):\n{}\n... ({} chars total)",
                preview,
                text.len()
            );
        } else {
            println!("Text:\n{}", text);
        }
    } else {
        println!("Text: (none)");
    }

    if !msg.attachments.is_empty() {
        println!();
        println!("=== Attachments ({}) ===", msg.attachments.len());
        for att in &msg.attachments {
            let size_kb = att.size / 1024;
            println!("{} ({} KB) [{}]", att.filename, size_kb, att.content_type);
        }
    }
}

#[derive(Default)]
struct StatsTotals {
    accepted: u64,
    delivered: u64,
    failed: u64,
    failed_permanent: u64,
    failed_temporary: u64,
    opened: u64,
    clicked: u64,
    unsubscribed: u64,
    complained: u64,
}

impl StatsTotals {
    fn from_entries(entries: &[api::StatEntry]) -> Self {
        let mut totals = Self::default();
        for entry in entries {
            if let Some(s) = &entry.accepted {
                totals.accepted += s.total.unwrap_or(0);
            }
            if let Some(s) = &entry.delivered {
                totals.delivered += s.total.unwrap_or(0);
            }
            if let Some(s) = &entry.failed {
                totals.failed += s.total.unwrap_or(0);
                totals.failed_permanent += s.permanent.unwrap_or(0);
                totals.failed_temporary += s.temporary.unwrap_or(0);
            }
            if let Some(s) = &entry.opened {
                totals.opened += s.total.unwrap_or(0);
            }
            if let Some(s) = &entry.clicked {
                totals.clicked += s.total.unwrap_or(0);
            }
            if let Some(s) = &entry.unsubscribed {
                totals.unsubscribed += s.total.unwrap_or(0);
            }
            if let Some(s) = &entry.complained {
                totals.complained += s.total.unwrap_or(0);
            }
        }
        totals
    }
}

fn print_stats(resp: api::StatsResponse) {
    println!("Statistics from {} to {}", resp.start, resp.end);
    println!("Resolution: {}", resp.resolution);
    println!();

    let totals = StatsTotals::from_entries(&resp.stats);

    println!("{:<20} {:>10}", "Metric", "Count");
    println!("{}", "-".repeat(32));
    println!("{:<20} {:>10}", "Accepted", totals.accepted);
    println!("{:<20} {:>10}", "Delivered", totals.delivered);
    println!("{:<20} {:>10}", "Failed (total)", totals.failed);
    println!("{:<20} {:>10}", "  Permanent", totals.failed_permanent);
    println!("{:<20} {:>10}", "  Temporary", totals.failed_temporary);
    println!("{:<20} {:>10}", "Opened", totals.opened);
    println!("{:<20} {:>10}", "Clicked", totals.clicked);
    println!("{:<20} {:>10}", "Unsubscribed", totals.unsubscribed);
    println!("{:<20} {:>10}", "Complained", totals.complained);

    if totals.delivered > 0 {
        println!();
        let open_rate = (totals.opened as f64 / totals.delivered as f64) * 100.0;
        let click_rate = (totals.clicked as f64 / totals.delivered as f64) * 100.0;
        println!("Open rate:  {:.1}%", open_rate);
        println!("Click rate: {:.1}%", click_rate);
    }
}

async fn run_events(
    event: Option<String>,
    recipient: Option<String>,
    limit: u32,
    json: bool,
) -> Result<()> {
    let client = get_client()?;
    let resp = client
        .list_events(event.as_deref(), recipient.as_deref(), limit)
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp.items)?);
    } else {
        print_events(resp);
    }
    Ok(())
}

async fn run_message(storage_url: String, json: bool, headers: bool) -> Result<()> {
    let client = get_client()?;
    let msg = client.fetch_stored_message(&storage_url).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&msg)?);
    } else {
        print_message(&msg, headers);
    }
    Ok(())
}

async fn run_bounces(limit: u32, json: bool, command: Option<BouncesCommands>) -> Result<()> {
    let client = get_client()?;

    match command {
        Some(BouncesCommands::Delete { email }) => {
            client.delete_bounce(&email).await?;
            println!("Removed {} from bounce list", email);
        }
        None => {
            let resp = client.list_bounces(limit).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp.items)?);
            } else {
                print_bounces(resp);
            }
        }
    }
    Ok(())
}

async fn run_complaints(limit: u32, json: bool, command: Option<ComplaintsCommands>) -> Result<()> {
    let client = get_client()?;

    match command {
        Some(ComplaintsCommands::Delete { email }) => {
            client.delete_complaint(&email).await?;
            println!("Removed {} from complaints list", email);
        }
        None => {
            let resp = client.list_complaints(limit).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp.items)?);
            } else {
                print_complaints(resp);
            }
        }
    }
    Ok(())
}

async fn run_unsubscribes(
    limit: u32,
    json: bool,
    command: Option<UnsubscribesCommands>,
) -> Result<()> {
    let client = get_client()?;

    match command {
        Some(UnsubscribesCommands::Delete { email }) => {
            client.delete_unsubscribe(&email).await?;
            println!("Removed {} from unsubscribes list", email);
        }
        None => {
            let resp = client.list_unsubscribes(limit).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp.items)?);
            } else {
                print_unsubscribes(resp);
            }
        }
    }
    Ok(())
}

async fn run_stats(duration: String, json: bool) -> Result<()> {
    let client = get_client()?;
    let event_types = [
        "accepted",
        "delivered",
        "failed",
        "opened",
        "clicked",
        "unsubscribed",
        "complained",
    ];
    let resp = client.get_stats(&event_types, &duration).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        print_stats(resp);
    }
    Ok(())
}

async fn run_config(
    api_key: Option<String>,
    domain: Option<String>,
    region: Option<String>,
) -> Result<()> {
    let mut cfg = config::load_config().unwrap_or_default();

    if api_key.is_none() && domain.is_none() && region.is_none() {
        println!("Config file: ~/.config/mailgun-cli/config.toml");
        println!();
        if let Some(key) = &cfg.api_key {
            let masked = if key.len() > 8 {
                format!("{}...{}", &key[..4], &key[key.len() - 4..])
            } else {
                "*".repeat(key.len())
            };
            println!("API key: {}", masked);
        } else {
            println!("API key: (not set)");
        }
        println!("Domain:  {}", cfg.domain.as_deref().unwrap_or("(not set)"));
        println!(
            "Region:  {}",
            match cfg.region {
                Region::Us => "us",
                Region::Eu => "eu",
            }
        );
        return Ok(());
    }

    if let Some(key) = api_key {
        cfg.api_key = Some(key);
    }
    if let Some(d) = domain {
        cfg.domain = Some(d);
    }
    if let Some(r) = region {
        cfg.region = match r.to_lowercase().as_str() {
            "us" => Region::Us,
            "eu" => Region::Eu,
            _ => anyhow::bail!("Invalid region '{}'. Use 'us' or 'eu'.", r),
        };
    }

    config::save_config(&cfg)?;
    println!("Config saved to ~/.config/mailgun-cli/config.toml");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Events {
            event,
            recipient,
            limit,
            json,
        } => run_events(event, recipient, limit, json).await,
        Commands::Message {
            storage_url,
            json,
            headers,
        } => run_message(storage_url, json, headers).await,
        Commands::Bounces {
            limit,
            json,
            command,
        } => run_bounces(limit, json, command).await,
        Commands::Complaints {
            limit,
            json,
            command,
        } => run_complaints(limit, json, command).await,
        Commands::Unsubscribes {
            limit,
            json,
            command,
        } => run_unsubscribes(limit, json, command).await,
        Commands::Stats { duration, json } => run_stats(duration, json).await,
        Commands::Config {
            api_key,
            domain,
            region,
        } => run_config(api_key, domain, region).await,
    }
}
