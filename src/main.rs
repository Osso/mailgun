mod api;
mod config;

use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};

use config::{Region, SiteConfig};

const MESSAGE_PREVIEW_CHARS: usize = 500;

#[derive(Parser)]
#[command(name = "mailgun")]
#[command(about = "CLI tool to access Mailgun API")]
struct Cli {
    /// Site to use (from config). Defaults to the configured default site.
    #[arg(short, long, global = true)]
    site: Option<String>,
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
    /// List sending IP addresses (account or domain-assigned)
    Ips {
        /// Show details for a single IP address instead of listing
        ip: Option<String>,
        /// List all account IPs instead of just this domain's assigned IPs
        #[arg(short, long)]
        all: bool,
        /// Only show dedicated IPs (implies --all)
        #[arg(long)]
        dedicated: bool,
        /// Query a specific domain's assigned IPs instead of the configured one
        #[arg(long)]
        domain: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// List domains on the account
    Domains {
        /// Number of results to return
        #[arg(short = 'n', long, default_value = "100")]
        limit: u32,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Manage configured sites (credentials per Mailgun account/domain)
    Config {
        #[command(subcommand)]
        action: Option<ConfigCommands>,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Add or update a site's credentials
    Set {
        /// Site name (e.g., "globalcomix", "mangahelpers")
        name: String,
        /// Mailgun API key
        #[arg(short = 'k', long)]
        api_key: String,
        /// Mailgun domain (e.g., mg.example.com)
        #[arg(short, long)]
        domain: String,
        /// Region: us or eu
        #[arg(short, long, default_value = "us")]
        region: String,
        /// Set as the default site
        #[arg(long)]
        default: bool,
    },
    /// List configured sites
    List,
    /// Set the default site
    Default {
        /// Site name to set as default
        name: String,
    },
    /// Remove a site from config
    Remove {
        /// Site name to remove
        name: String,
    },
    /// Show config file path
    Path,
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

fn get_client(site: Option<&str>) -> Result<api::Client> {
    let cfg = config::load_config()?;
    let resolved = cfg.resolve_site(site)?;
    #[cfg(test)]
    if let Ok(base_url) = std::env::var("MAILGUN_TEST_BASE_URL") {
        return api::Client::with_base_url(&resolved.api_key, &resolved.domain, base_url);
    }
    api::Client::new(&resolved.api_key, &resolved.domain, resolved.region)
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

fn print_message_metadata(msg: &api::StoredMessage) {
    if let Some(subject) = &msg.subject {
        println!("Subject: {}", subject);
    }
    if let Some(from) = &msg.from {
        println!("From: {}", from);
    }
    if let Some(to) = &msg.to {
        println!("To: {}", to);
    }
}

fn print_message_text(text: Option<&str>) {
    let Some(text) = text else {
        println!("Text: (none)");
        return;
    };

    let preview: String = text.chars().take(MESSAGE_PREVIEW_CHARS).collect();
    if text.len() > MESSAGE_PREVIEW_CHARS {
        println!(
            "Text (truncated):\n{}\n... ({} chars total)",
            preview,
            text.len()
        );
    } else {
        println!("Text:\n{}", text);
    }
}

fn print_message_attachments(attachments: &[api::AttachmentInfo]) {
    if attachments.is_empty() {
        return;
    }

    println!();
    println!("=== Attachments ({}) ===", attachments.len());
    for att in attachments {
        let size_kb = att.size / 1024;
        println!("{} ({} KB) [{}]", att.filename, size_kb, att.content_type);
    }
}

fn print_message(msg: &api::StoredMessage, headers_only: bool) {
    if headers_only {
        println!("=== Headers ===");
        print_headers(&msg.headers);
        return;
    }

    println!("=== Message Details ===");
    print_message_metadata(msg);

    println!();
    println!("=== Headers ===");
    print_headers(&msg.headers);

    println!();
    println!("=== Body ===");
    print_message_text(msg.stripped_text.as_deref());
    print_message_attachments(&msg.attachments);
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
    site: Option<&str>,
    event: Option<String>,
    recipient: Option<String>,
    limit: u32,
    json: bool,
) -> Result<()> {
    let client = get_client(site)?;
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

async fn run_message(
    site: Option<&str>,
    storage_url: String,
    json: bool,
    headers: bool,
) -> Result<()> {
    let client = get_client(site)?;
    let msg = client.fetch_stored_message(&storage_url).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&msg)?);
    } else {
        print_message(&msg, headers);
    }
    Ok(())
}

async fn run_bounces(
    site: Option<&str>,
    limit: u32,
    json: bool,
    command: Option<BouncesCommands>,
) -> Result<()> {
    let client = get_client(site)?;

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

async fn run_complaints(
    site: Option<&str>,
    limit: u32,
    json: bool,
    command: Option<ComplaintsCommands>,
) -> Result<()> {
    let client = get_client(site)?;

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
    site: Option<&str>,
    limit: u32,
    json: bool,
    command: Option<UnsubscribesCommands>,
) -> Result<()> {
    let client = get_client(site)?;

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

fn print_ips(resp: &api::IpsResponse) {
    if resp.items.is_empty() {
        println!("No IP addresses found.");
        return;
    }
    for ip in &resp.items {
        println!("{}", ip);
    }
    if let Some(total) = resp.total_count {
        println!();
        println!("Total: {}", total);
    }
}

fn print_ip_details(ip: &api::IpDetails) {
    println!("IP:        {}", ip.ip);
    println!("RDNS:      {}", ip.rdns.as_deref().unwrap_or("-"));
    let dedicated = match ip.dedicated {
        Some(true) => "yes",
        Some(false) => "no",
        None => "-",
    };
    println!("Dedicated: {}", dedicated);
}

async fn run_ips(
    site: Option<&str>,
    ip: Option<String>,
    all: bool,
    dedicated: bool,
    domain: Option<String>,
    json: bool,
) -> Result<()> {
    let client = get_client(site)?;

    if let Some(ip) = ip {
        let details = client.get_ip(&ip).await?;
        if json {
            println!("{}", serde_json::to_string_pretty(&details)?);
        } else {
            print_ip_details(&details);
        }
        return Ok(());
    }

    // `--dedicated` is an account-level filter, so it implies `--all`.
    let resp = if all || dedicated {
        client.list_account_ips(dedicated).await?
    } else {
        client.list_domain_ips(domain.as_deref()).await?
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        print_ips(&resp);
    }
    Ok(())
}

fn print_domains(resp: &api::DomainsResponse) {
    if resp.items.is_empty() {
        println!("No domains found.");
        return;
    }

    println!("{:<32} {:<10} {:<10} CREATED", "NAME", "STATE", "TYPE");
    println!("{}", "-".repeat(80));

    for domain in &resp.items {
        let name = if domain.is_disabled {
            format!("{} (disabled)", domain.name)
        } else {
            domain.name.clone()
        };
        println!(
            "{:<32} {:<10} {:<10} {}",
            name,
            domain.state.as_deref().unwrap_or("-"),
            domain.domain_type.as_deref().unwrap_or("-"),
            domain.created_at.as_deref().unwrap_or("-"),
        );
    }
}

async fn run_domains(site: Option<&str>, limit: u32, json: bool) -> Result<()> {
    let client = get_client(site)?;
    let resp = client.list_domains(limit).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        print_domains(&resp);
    }
    Ok(())
}

async fn run_stats(site: Option<&str>, duration: String, json: bool) -> Result<()> {
    let client = get_client(site)?;
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

fn run_config(action: Option<ConfigCommands>) -> Result<()> {
    match action {
        None | Some(ConfigCommands::List) => config_list(),
        Some(ConfigCommands::Set {
            name,
            api_key,
            domain,
            region,
            default,
        }) => config_set(name, api_key, domain, region, default),
        Some(ConfigCommands::Default { name }) => config_set_default(name),
        Some(ConfigCommands::Remove { name }) => config_remove(name),
        Some(ConfigCommands::Path) => {
            println!("{}", config::config_path_display());
            Ok(())
        }
    }
}

fn config_set(
    name: String,
    api_key: String,
    domain: String,
    region: String,
    default: bool,
) -> Result<()> {
    let mut cfg = config::load_config().unwrap_or_default();
    let site = SiteConfig {
        api_key,
        domain,
        region: parse_region(&region)?,
    };
    cfg.set_site(&name, site, default);
    config::save_config(&cfg)?;
    let marker = if cfg.default_site.as_deref() == Some(&name) {
        " (default)"
    } else {
        ""
    };
    println!(
        "Site '{}'{} saved to {}",
        name,
        marker,
        config::config_path_display()
    );
    Ok(())
}

fn config_list() -> Result<()> {
    let cfg = config::load_config()?;
    println!("Config file: {}", config::config_path_display());
    println!();

    if cfg.sites.is_empty() {
        if cfg.api_key.is_some() && cfg.domain.is_some() {
            println!("(legacy single-site config)");
            println!(
                "  domain:  {}",
                cfg.domain.as_deref().unwrap_or("(not set)")
            );
            println!("  api key: {}", format_api_key(cfg.api_key.as_deref()));
            println!("  region:  {}", region_label(cfg.region));
            println!();
            println!("Migrate with: mailgun config set <name> -k <key> -d <domain>");
        } else {
            println!("No sites configured.");
            println!("Add one with: mailgun config set <name> -k <key> -d <domain>");
        }
        return Ok(());
    }

    let mut names: Vec<_> = cfg.sites.keys().cloned().collect();
    names.sort();
    for name in names {
        let site = &cfg.sites[&name];
        let marker = if cfg.default_site.as_deref() == Some(&name) {
            " *"
        } else {
            ""
        };
        println!("{}{}", name, marker);
        println!("  domain:  {}", site.domain);
        println!("  api key: {}", format_api_key(Some(&site.api_key)));
        println!("  region:  {}", region_label(site.region));
    }
    Ok(())
}

fn config_set_default(name: String) -> Result<()> {
    let mut cfg = config::load_config()?;
    if !cfg.sites.contains_key(&name) {
        anyhow::bail!("Site '{}' not found. Available: {}", name, cfg.list_sites());
    }
    cfg.default_site = Some(name.clone());
    config::save_config(&cfg)?;
    println!("Default site set to '{}'", name);
    Ok(())
}

fn config_remove(name: String) -> Result<()> {
    let mut cfg = config::load_config()?;
    if cfg.remove_site(&name) {
        config::save_config(&cfg)?;
        println!("Site '{}' removed", name);
    } else {
        println!("Site '{}' not found", name);
    }
    Ok(())
}

fn format_api_key(api_key: Option<&str>) -> String {
    let Some(key) = api_key else {
        return "(not set)".to_string();
    };
    if key.len() > 8 {
        return format!("{}...{}", &key[..4], &key[key.len() - 4..]);
    }
    "*".repeat(key.len())
}

fn region_label(region: Region) -> &'static str {
    match region {
        Region::Us => "us",
        Region::Eu => "eu",
    }
}

fn parse_region(region: &str) -> Result<Region> {
    match region.to_lowercase().as_str() {
        "us" => Ok(Region::Us),
        "eu" => Ok(Region::Eu),
        _ => anyhow::bail!("Invalid region '{}'. Use 'us' or 'eu'.", region),
    }
}

async fn run_command(command: Commands, site: Option<&str>) -> Result<()> {
    match command {
        Commands::Events {
            event,
            recipient,
            limit,
            json,
        } => run_events(site, event, recipient, limit, json).await,
        Commands::Message {
            storage_url,
            json,
            headers,
        } => run_message(site, storage_url, json, headers).await,
        Commands::Bounces { .. } | Commands::Complaints { .. } | Commands::Unsubscribes { .. } => {
            run_suppression_command(command, site).await
        }
        Commands::Stats { duration, json } => run_stats(site, duration, json).await,
        Commands::Ips {
            ip,
            all,
            dedicated,
            domain,
            json,
        } => run_ips(site, ip, all, dedicated, domain, json).await,
        Commands::Domains { limit, json } => run_domains(site, limit, json).await,
        Commands::Config { action } => run_config(action),
    }
}

async fn run_suppression_command(command: Commands, site: Option<&str>) -> Result<()> {
    match command {
        Commands::Bounces {
            limit,
            json,
            command,
        } => run_bounces(site, limit, json, command).await,
        Commands::Complaints {
            limit,
            json,
            command,
        } => run_complaints(site, limit, json, command).await,
        Commands::Unsubscribes {
            limit,
            json,
            command,
        } => run_unsubscribes(site, limit, json, command).await,
        _ => unreachable!("only suppression commands are delegated here"),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    run_command(cli.command, cli.site.as_deref()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::net::SocketAddr;
    use std::sync::{Mutex, OnceLock};
    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn with_temp_config(test: impl FnOnce()) {
        let guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let dir = tempdir().unwrap();

        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", dir.path());
        }

        test();

        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        drop(guard);
    }

    struct CliMockServer {
        base_url: String,
        requests: std::sync::Arc<Mutex<Vec<String>>>,
    }

    impl CliMockServer {
        async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let requests = std::sync::Arc::new(Mutex::new(Vec::new()));
            let server_requests = std::sync::Arc::clone(&requests);

            tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        break;
                    };
                    let requests = std::sync::Arc::clone(&server_requests);
                    tokio::spawn(async move {
                        let mut buffer = vec![0; 4096];
                        let bytes_read = stream.read(&mut buffer).await.unwrap();
                        let request = String::from_utf8_lossy(&buffer[..bytes_read]);
                        let request_line = request.lines().next().unwrap_or_default().to_string();
                        let body = cli_response_body(&request_line);
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

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    fn display_addr(addr: SocketAddr) -> String {
        format!("{}:{}", addr.ip(), addr.port())
    }

    fn cli_response_body(request_line: &str) -> &'static str {
        let path = request_line.split_whitespace().nth(1).unwrap_or_default();
        cli_response_for_path(path).unwrap_or(r#"{}"#)
    }

    fn cli_response_for_path(path: &str) -> Option<&'static str> {
        cli_suppression_response(path)
            .or_else(|| cli_stats_response(path))
            .or_else(|| cli_message_response(path))
            .or_else(|| cli_ip_response(path))
            .or_else(|| cli_domain_response(path))
    }

    fn cli_suppression_response(path: &str) -> Option<&'static str> {
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

    fn cli_stats_response(path: &str) -> Option<&'static str> {
        let stats_path = "/v3/example.com/stats/total?event=accepted,delivered,failed,opened,clicked,unsubscribed,complained&duration=7d";
        if path != stats_path {
            return None;
        }
        Some(
            r#"{"start":"2026-01-01","end":"2026-01-08","resolution":"day","stats":[{"time":"2026-01-01","accepted":{"total":3},"delivered":{"total":2}}]}"#,
        )
    }

    fn cli_message_response(path: &str) -> Option<&'static str> {
        match path {
            "/v3/message" => Some(
                r#"{"message-headers":[["Received","mx1"]],"From":"sender@example.com","To":"reader@example.com","Subject":"Stored","stripped-text":"Body","attachments":[]}"#,
            ),
            _ => None,
        }
    }

    fn cli_ip_response(path: &str) -> Option<&'static str> {
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

    fn cli_domain_response(path: &str) -> Option<&'static str> {
        match path {
            "/v4/domains?limit=5" => Some(
                r#"{"items":[{"name":"example.com","state":"active","type":"sandbox","is_disabled":false,"created_at":"2026-01-01"}],"total_count":1}"#,
            ),
            _ => None,
        }
    }

    fn configure_mocked_site(base_url: &str) -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", dir.path());
            std::env::set_var("MAILGUN_TEST_BASE_URL", base_url);
        }
        run_config(Some(ConfigCommands::Set {
            name: "prod".to_string(),
            api_key: "key-prod".to_string(),
            domain: "example.com".to_string(),
            region: "us".to_string(),
            default: true,
        }))
        .unwrap();
        dir
    }

    fn clear_mocked_site_env() {
        unsafe {
            std::env::remove_var("MAILGUN_TEST_BASE_URL");
            std::env::remove_var("XDG_CONFIG_HOME");
        }
    }

    #[test]
    fn parses_global_site_and_nested_delete_commands() {
        let cli = Cli::try_parse_from([
            "mailgun",
            "--site",
            "prod",
            "bounces",
            "--json",
            "delete",
            "bad@example.com",
        ])
        .unwrap();

        assert_eq!(cli.site.as_deref(), Some("prod"));
        match cli.command {
            Commands::Bounces {
                limit,
                json,
                command: Some(BouncesCommands::Delete { email }),
            } => {
                assert_eq!(limit, 20);
                assert!(json);
                assert_eq!(email, "bad@example.com");
            }
            _ => panic!("expected bounces delete command"),
        }
    }

    #[test]
    fn parses_events_message_stats_ips_domains_and_config_commands() {
        let events = Cli::try_parse_from([
            "mailgun",
            "events",
            "--event",
            "failed",
            "--recipient",
            "reader@example.com",
            "-n",
            "25",
            "--json",
        ])
        .unwrap();
        let message =
            Cli::try_parse_from(["mailgun", "message", "https://storage", "--headers"]).unwrap();
        let stats = Cli::try_parse_from(["mailgun", "stats", "--duration", "7d"]).unwrap();
        let ips =
            Cli::try_parse_from(["mailgun", "ips", "--all", "--dedicated", "--json"]).unwrap();
        let domains = Cli::try_parse_from(["mailgun", "domains", "-n", "50"]).unwrap();
        let config = Cli::try_parse_from([
            "mailgun",
            "config",
            "set",
            "prod",
            "--api-key",
            "key",
            "--domain",
            "example.com",
            "--region",
            "eu",
            "--default",
        ])
        .unwrap();

        match events.command {
            Commands::Events {
                event,
                recipient,
                limit,
                json,
            } => {
                assert_eq!(event.as_deref(), Some("failed"));
                assert_eq!(recipient.as_deref(), Some("reader@example.com"));
                assert_eq!(limit, 25);
                assert!(json);
            }
            _ => panic!("expected events command"),
        }
        match message.command {
            Commands::Message {
                storage_url,
                json,
                headers,
            } => {
                assert_eq!(storage_url, "https://storage");
                assert!(!json);
                assert!(headers);
            }
            _ => panic!("expected message command"),
        }
        match stats.command {
            Commands::Stats { duration, json } => {
                assert_eq!(duration, "7d");
                assert!(!json);
            }
            _ => panic!("expected stats command"),
        }
        match ips.command {
            Commands::Ips {
                ip,
                all,
                dedicated,
                domain,
                json,
            } => {
                assert_eq!(ip, None);
                assert!(all);
                assert!(dedicated);
                assert_eq!(domain, None);
                assert!(json);
            }
            _ => panic!("expected ips command"),
        }
        match domains.command {
            Commands::Domains { limit, json } => {
                assert_eq!(limit, 50);
                assert!(!json);
            }
            _ => panic!("expected domains command"),
        }
        match config.command {
            Commands::Config {
                action:
                    Some(ConfigCommands::Set {
                        name,
                        api_key,
                        domain,
                        region,
                        default,
                    }),
            } => {
                assert_eq!(name, "prod");
                assert_eq!(api_key, "key");
                assert_eq!(domain, "example.com");
                assert_eq!(region, "eu");
                assert!(default);
            }
            _ => panic!("expected config set command"),
        }
    }

    #[test]
    fn formats_timestamps_truncation_and_delivery_status() {
        assert_eq!(format_timestamp(0.0), "1970-01-01 00:00:00 UTC");
        assert_eq!(truncate("abcdef", 3), "abc...");
        assert_eq!(truncate("éclair", 2), "éc...");
        assert_eq!(truncate("short", 10), "short");

        let status = api::DeliveryStatus {
            code: Some(550),
            message: Some("Mailbox unavailable because recipient is disabled".to_string()),
            description: None,
        };
        assert_eq!(
            format_delivery_status(&status),
            " [550] Mailbox unavailable because recipient is disabled"
        );

        let code_only = api::DeliveryStatus {
            code: Some(250),
            message: None,
            description: None,
        };
        let message_only = api::DeliveryStatus {
            code: None,
            message: Some("queued".to_string()),
            description: None,
        };
        let empty = api::DeliveryStatus {
            code: None,
            message: None,
            description: None,
        };
        assert_eq!(format_delivery_status(&code_only), " [250]");
        assert_eq!(format_delivery_status(&message_only), " queued");
        assert_eq!(format_delivery_status(&empty), "");
    }

    #[test]
    fn aggregates_stats_totals_with_missing_counts_as_zero() {
        let entries = vec![
            api::StatEntry {
                time: "2026-01-01".to_string(),
                accepted: Some(api::StatCount {
                    total: Some(10),
                    permanent: None,
                    temporary: None,
                }),
                delivered: Some(api::StatCount {
                    total: Some(7),
                    permanent: None,
                    temporary: None,
                }),
                failed: Some(api::StatCount {
                    total: Some(3),
                    permanent: Some(2),
                    temporary: Some(1),
                }),
                opened: Some(api::StatCount {
                    total: Some(4),
                    permanent: None,
                    temporary: None,
                }),
                clicked: Some(api::StatCount {
                    total: Some(2),
                    permanent: None,
                    temporary: None,
                }),
                unsubscribed: Some(api::StatCount {
                    total: Some(1),
                    permanent: None,
                    temporary: None,
                }),
                complained: None,
                stored: None,
            },
            api::StatEntry {
                time: "2026-01-02".to_string(),
                accepted: Some(api::StatCount {
                    total: None,
                    permanent: None,
                    temporary: None,
                }),
                delivered: None,
                failed: None,
                opened: None,
                clicked: None,
                unsubscribed: None,
                complained: Some(api::StatCount {
                    total: Some(1),
                    permanent: None,
                    temporary: None,
                }),
                stored: Some(api::StatCount {
                    total: Some(5),
                    permanent: None,
                    temporary: None,
                }),
            },
        ];

        let totals = StatsTotals::from_entries(&entries);

        assert_eq!(totals.accepted, 10);
        assert_eq!(totals.delivered, 7);
        assert_eq!(totals.failed, 3);
        assert_eq!(totals.failed_permanent, 2);
        assert_eq!(totals.failed_temporary, 1);
        assert_eq!(totals.opened, 4);
        assert_eq!(totals.clicked, 2);
        assert_eq!(totals.unsubscribed, 1);
        assert_eq!(totals.complained, 1);
    }

    #[test]
    fn formats_api_keys_and_regions() {
        assert_eq!(format_api_key(None), "(not set)");
        assert_eq!(format_api_key(Some("short")), "*****");
        assert_eq!(format_api_key(Some("key-123456789")), "key-...6789");

        assert_eq!(region_label(Region::Us), "us");
        assert_eq!(region_label(Region::Eu), "eu");
        assert_eq!(parse_region("US").unwrap(), Region::Us);
        assert_eq!(parse_region("eu").unwrap(), Region::Eu);
        assert!(parse_region("ap").is_err());
    }

    #[test]
    fn config_commands_persist_default_and_removed_sites() {
        with_temp_config(|| {
            run_config(Some(ConfigCommands::Set {
                name: "prod".to_string(),
                api_key: "key-prod".to_string(),
                domain: "mg.example.com".to_string(),
                region: "eu".to_string(),
                default: true,
            }))
            .unwrap();
            run_config(Some(ConfigCommands::Set {
                name: "dev".to_string(),
                api_key: "key-dev".to_string(),
                domain: "dev.example.com".to_string(),
                region: "us".to_string(),
                default: false,
            }))
            .unwrap();

            let cfg = config::load_config().unwrap();
            assert_eq!(cfg.default_site.as_deref(), Some("prod"));
            assert_eq!(cfg.sites["prod"].region, Region::Eu);
            assert_eq!(cfg.sites["dev"].domain, "dev.example.com");

            run_config(Some(ConfigCommands::Default {
                name: "dev".to_string(),
            }))
            .unwrap();
            assert_eq!(
                config::load_config().unwrap().default_site.as_deref(),
                Some("dev")
            );

            run_config(Some(ConfigCommands::Remove {
                name: "prod".to_string(),
            }))
            .unwrap();
            let cfg = config::load_config().unwrap();
            assert!(!cfg.sites.contains_key("prod"));
            assert_eq!(cfg.default_site.as_deref(), Some("dev"));
        });
    }

    #[test]
    fn config_commands_handle_list_path_missing_remove_and_bad_default() {
        with_temp_config(|| {
            run_config(None).unwrap();
            run_config(Some(ConfigCommands::List)).unwrap();
            run_config(Some(ConfigCommands::Path)).unwrap();

            run_config(Some(ConfigCommands::Remove {
                name: "missing".to_string(),
            }))
            .unwrap();
            assert!(config::load_config().unwrap().sites.is_empty());

            let err = run_config(Some(ConfigCommands::Default {
                name: "missing".to_string(),
            }))
            .unwrap_err();
            assert!(err.to_string().contains("Site 'missing' not found"));
        });
    }

    #[test]
    fn print_functions_handle_empty_and_populated_payloads() {
        let result = std::panic::catch_unwind(|| {
            print_events(api::EventsResponse {
                items: Vec::new(),
                paging: None,
            });
            print_events(api::EventsResponse {
                items: vec![
                    api::Event {
                        id: Some("evt-1".to_string()),
                        event: "failed".to_string(),
                        timestamp: 1710000000.0,
                        recipient: Some("reader@example.com".to_string()),
                        message: Some(api::MessageInfo {
                            headers: Some(api::MessageHeaders {
                                message_id: Some("msg-1".to_string()),
                                subject: Some(
                                    "A subject that is intentionally longer than forty characters"
                                        .to_string(),
                                ),
                                from: Some("sender@example.com".to_string()),
                                to: Some("reader@example.com".to_string()),
                            }),
                        }),
                        tags: vec!["news".to_string()],
                        delivery_status: Some(api::DeliveryStatus {
                            code: Some(550),
                            message: Some("Mailbox unavailable".to_string()),
                            description: None,
                        }),
                        reason: None,
                        severity: Some("permanent".to_string()),
                        storage: Some(api::StorageInfo {
                            url: "https://storage".to_string(),
                        }),
                    },
                    api::Event {
                        id: None,
                        event: "rejected".to_string(),
                        timestamp: 1710000001.0,
                        recipient: None,
                        message: None,
                        tags: Vec::new(),
                        delivery_status: None,
                        reason: Some("policy".to_string()),
                        severity: None,
                        storage: None,
                    },
                ],
                paging: None,
            });

            print_bounces(api::BouncesResponse {
                items: Vec::new(),
                paging: None,
            });
            print_bounces(api::BouncesResponse {
                items: vec![api::Bounce {
                    address: "bad@example.com".to_string(),
                    code: "550".to_string(),
                    error: "blocked because this address repeatedly failed delivery".to_string(),
                    created_at: "2026-01-01".to_string(),
                }],
                paging: None,
            });

            print_complaints(api::ComplaintsResponse {
                items: Vec::new(),
                paging: None,
            });
            print_complaints(api::ComplaintsResponse {
                items: vec![api::Complaint {
                    address: "spam@example.com".to_string(),
                    created_at: "2026-01-02".to_string(),
                }],
                paging: None,
            });

            print_unsubscribes(api::UnsubscribesResponse {
                items: Vec::new(),
                paging: None,
            });
            print_unsubscribes(api::UnsubscribesResponse {
                items: vec![
                    api::Unsubscribe {
                        address: "tagged@example.com".to_string(),
                        tags: vec!["newsletter".to_string(), "promo".to_string()],
                        created_at: "2026-01-03".to_string(),
                    },
                    api::Unsubscribe {
                        address: "untagged@example.com".to_string(),
                        tags: Vec::new(),
                        created_at: "2026-01-04".to_string(),
                    },
                ],
                paging: None,
            });

            print_ips(&api::IpsResponse {
                items: Vec::new(),
                total_count: None,
            });
            print_ips(&api::IpsResponse {
                items: vec!["1.2.3.4".to_string()],
                total_count: Some(1),
            });
            print_ip_details(&api::IpDetails {
                ip: "1.2.3.4".to_string(),
                rdns: Some("mail.example.com".to_string()),
                dedicated: Some(true),
            });
            print_ip_details(&api::IpDetails {
                ip: "5.6.7.8".to_string(),
                rdns: None,
                dedicated: Some(false),
            });
            print_ip_details(&api::IpDetails {
                ip: "9.9.9.9".to_string(),
                rdns: None,
                dedicated: None,
            });

            print_domains(&api::DomainsResponse {
                items: Vec::new(),
                total_count: None,
            });
            print_domains(&api::DomainsResponse {
                items: vec![
                    api::Domain {
                        name: "example.com".to_string(),
                        state: Some("active".to_string()),
                        domain_type: Some("custom".to_string()),
                        is_disabled: false,
                        created_at: Some("2026-01-01".to_string()),
                    },
                    api::Domain {
                        name: "disabled.example.com".to_string(),
                        state: None,
                        domain_type: None,
                        is_disabled: true,
                        created_at: None,
                    },
                ],
                total_count: Some(2),
            });
        });

        assert!(result.is_ok());
    }

    #[test]
    fn print_message_and_stats_handle_optional_and_truncated_fields() {
        let long_text = "x".repeat(MESSAGE_PREVIEW_CHARS + 10);
        let result = std::panic::catch_unwind(|| {
            let message = api::StoredMessage {
                headers: api::StoredMessageHeaders {
                    received: vec!["mx1".to_string(), "mx2".to_string()],
                    dkim: Some("dkim".to_string()),
                    mime_version: Some("1.0".to_string()),
                    content_transfer_encoding: Some("quoted-printable".to_string()),
                    list_unsubscribe: Some("<mailto:unsubscribe@example.com>".to_string()),
                    list_unsubscribe_post: Some("List-Unsubscribe=One-Click".to_string()),
                    other: vec![("X-Campaign".to_string(), "summer".to_string())],
                },
                from: Some("sender@example.com".to_string()),
                to: Some("reader@example.com".to_string()),
                subject: Some("Stored".to_string()),
                stripped_text: Some(long_text),
                stripped_html: Some("<p>Body</p>".to_string()),
                stripped_signature: None,
                attachments: vec![api::AttachmentInfo {
                    filename: "report.pdf".to_string(),
                    size: 2048,
                    content_type: "application/pdf".to_string(),
                }],
            };

            print_message(&message, true);
            print_message(&message, false);
            print_message_text(None);
            print_message_text(Some("short body"));

            print_stats(api::StatsResponse {
                start: "2026-01-01".to_string(),
                end: "2026-01-08".to_string(),
                resolution: "day".to_string(),
                stats: vec![api::StatEntry {
                    time: "2026-01-01".to_string(),
                    accepted: Some(api::StatCount {
                        total: Some(10),
                        permanent: None,
                        temporary: None,
                    }),
                    delivered: Some(api::StatCount {
                        total: Some(5),
                        permanent: None,
                        temporary: None,
                    }),
                    failed: Some(api::StatCount {
                        total: Some(1),
                        permanent: Some(1),
                        temporary: Some(0),
                    }),
                    opened: Some(api::StatCount {
                        total: Some(2),
                        permanent: None,
                        temporary: None,
                    }),
                    clicked: Some(api::StatCount {
                        total: Some(1),
                        permanent: None,
                        temporary: None,
                    }),
                    unsubscribed: Some(api::StatCount {
                        total: Some(1),
                        permanent: None,
                        temporary: None,
                    }),
                    complained: Some(api::StatCount {
                        total: Some(1),
                        permanent: None,
                        temporary: None,
                    }),
                    stored: None,
                }],
            });
        });

        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_functions_call_expected_mailgun_endpoints() {
        let guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let server = CliMockServer::start().await;
        let _dir = configure_mocked_site(&server.base_url);

        run_event_and_message_paths(&server).await;
        run_suppression_paths().await;
        run_ip_domain_and_stats_paths().await;
        assert_run_function_requests(&server.requests());

        clear_mocked_site_env();
        drop(guard);
    }

    async fn run_event_and_message_paths(server: &CliMockServer) {
        run_events(
            None,
            Some("failed".to_string()),
            Some("reader@example.com".to_string()),
            2,
            false,
        )
        .await
        .unwrap();
        run_events(
            None,
            Some("failed".to_string()),
            Some("reader@example.com".to_string()),
            2,
            true,
        )
        .await
        .unwrap();
        run_message(None, format!("{}/message", server.base_url), false, false)
            .await
            .unwrap();
        run_message(None, format!("{}/message", server.base_url), true, true)
            .await
            .unwrap();
    }

    async fn run_suppression_paths() {
        run_bounces(None, 1, false, None).await.unwrap();
        run_bounces(
            None,
            1,
            true,
            Some(BouncesCommands::Delete {
                email: "bad@example.com".to_string(),
            }),
        )
        .await
        .unwrap();
        run_complaints(None, 1, false, None).await.unwrap();
        run_complaints(
            None,
            1,
            true,
            Some(ComplaintsCommands::Delete {
                email: "spam@example.com".to_string(),
            }),
        )
        .await
        .unwrap();
        run_unsubscribes(None, 1, false, None).await.unwrap();
        run_unsubscribes(
            None,
            1,
            true,
            Some(UnsubscribesCommands::Delete {
                email: "gone@example.com".to_string(),
            }),
        )
        .await
        .unwrap();
    }

    async fn run_ip_domain_and_stats_paths() {
        run_ips(None, None, false, false, None, false)
            .await
            .unwrap();
        run_ips(None, None, true, false, None, true).await.unwrap();
        run_ips(None, None, false, true, None, false).await.unwrap();
        run_ips(
            None,
            None,
            false,
            false,
            Some("other.com".to_string()),
            false,
        )
        .await
        .unwrap();
        run_ips(None, Some("1.2.3.4".to_string()), false, false, None, false)
            .await
            .unwrap();
        run_ips(None, Some("1.2.3.4".to_string()), false, false, None, true)
            .await
            .unwrap();
        run_domains(None, 5, false).await.unwrap();
        run_domains(None, 5, true).await.unwrap();
        run_stats(None, "7d".to_string(), false).await.unwrap();
        run_stats(None, "7d".to_string(), true).await.unwrap();
    }

    fn assert_run_function_requests(requests: &[String]) {
        assert!(requests.contains(
            &"GET /v3/example.com/events?limit=2&event=failed&recipient=reader@example.com HTTP/1.1"
                .to_string()
        ));
        assert!(requests.contains(&"GET /v3/message HTTP/1.1".to_string()));
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
        assert!(
            requests.contains(&"GET /v3/example.com/stats/total?event=accepted,delivered,failed,opened,clicked,unsubscribed,complained&duration=7d HTTP/1.1".to_string())
        );
        assert!(requests.contains(&"GET /v3/domains/example.com/ips HTTP/1.1".to_string()));
        assert!(requests.contains(&"GET /v3/domains/other.com/ips HTTP/1.1".to_string()));
        assert!(requests.contains(&"GET /v3/ips HTTP/1.1".to_string()));
        assert!(requests.contains(&"GET /v3/ips?dedicated=true HTTP/1.1".to_string()));
        assert!(requests.contains(&"GET /v3/ips/1.2.3.4 HTTP/1.1".to_string()));
        assert!(requests.contains(&"GET /v4/domains?limit=5 HTTP/1.1".to_string()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_command_dispatches_config_and_api_commands() {
        let guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let server = CliMockServer::start().await;
        let _dir = configure_mocked_site(&server.base_url);

        run_core_commands(&server).await;
        run_collection_commands().await;
        run_report_commands().await;
        assert_command_requests(&server.requests());

        clear_mocked_site_env();
        drop(guard);
    }

    async fn run_core_commands(server: &CliMockServer) {
        run_command(
            Commands::Events {
                event: Some("failed".to_string()),
                recipient: Some("reader@example.com".to_string()),
                limit: 2,
                json: false,
            },
            None,
        )
        .await
        .unwrap();
        run_command(
            Commands::Message {
                storage_url: format!("{}/message", server.base_url),
                json: false,
                headers: true,
            },
            None,
        )
        .await
        .unwrap();
    }

    async fn run_collection_commands() {
        run_command(
            Commands::Bounces {
                limit: 1,
                json: false,
                command: None,
            },
            None,
        )
        .await
        .unwrap();
        run_command(
            Commands::Complaints {
                limit: 1,
                json: false,
                command: None,
            },
            None,
        )
        .await
        .unwrap();
        run_command(
            Commands::Unsubscribes {
                limit: 1,
                json: false,
                command: None,
            },
            None,
        )
        .await
        .unwrap();
    }

    async fn run_report_commands() {
        run_command(
            Commands::Ips {
                ip: Some("1.2.3.4".to_string()),
                all: false,
                dedicated: false,
                domain: None,
                json: false,
            },
            None,
        )
        .await
        .unwrap();
        run_command(
            Commands::Domains {
                limit: 5,
                json: false,
            },
            None,
        )
        .await
        .unwrap();
        run_command(
            Commands::Stats {
                duration: "7d".to_string(),
                json: false,
            },
            None,
        )
        .await
        .unwrap();
        run_command(
            Commands::Config {
                action: Some(ConfigCommands::Path),
            },
            None,
        )
        .await
        .unwrap();
    }

    fn assert_command_requests(requests: &[String]) {
        assert!(requests.iter().any(|request| request.contains("/events")));
        assert!(requests.iter().any(|request| request.contains("/message")));
        assert!(requests.iter().any(|request| request.contains("/bounces")));
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/complaints"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/unsubscribes"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/ips/1.2.3.4"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/domains?limit=5"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/stats/total"))
        );
    }
}
