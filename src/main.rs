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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    run_command(cli.command, cli.site.as_deref()).await
}
