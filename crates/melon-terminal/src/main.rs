//! `melon-terminal` — the merchant terminal binary.
//!
//! Two modes share the same reader/relay logic (in the `melon_terminal`
//! library):
//!
//! * **one-shot CLI** (no subcommand): wait for a card, authenticate, run one
//!   operation, print the result, exit. Handy for scripting and hardware
//!   bring-up.
//! * **`serve`**: a long-running local Web UI kiosk that owns the reader and
//!   serves a touch UI at `http://<bind>/`.
//!
//! Requires a physical reader, so it is exercised against hardware, not in CI.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use clap::{Args, Parser, Subcommand};
use serde_json::Value;
use tracing_subscriber::EnvFilter;

use melon_terminal::{
    Config, Op, WaitAbort, authenticate, fetch_system_codes, http_client, make_target,
    open_reader_auto, parse_u16, resolve_card, run_operation, wait_for_card,
};

#[derive(Parser, Debug)]
#[command(name = "melon-terminal", version, about)]
struct Cli {
    #[command(flatten)]
    conn: ConnArgs,

    #[command(flatten)]
    once: OnceArgs,

    #[command(subcommand)]
    command: Option<Command>,
}

/// Connection/reader options shared by every mode.
#[derive(Args, Debug)]
struct ConnArgs {
    /// Base URL of the melon server.
    #[arg(long, env = "MELON_SERVER", default_value = "http://127.0.0.1:8080")]
    server: String,

    /// Merchant API key (bearer secret).
    #[arg(long, env = "MELON_API_KEY")]
    api_key: String,

    /// Override the usable FeliCa system codes instead of fetching them from the
    /// server. Hex (`0x0003`) or decimal; repeat the flag or comma-separate.
    /// Normally omitted — the server reports which systems it holds keys for.
    /// Area and service are fixed at 0x0000.
    #[arg(long = "system-code", value_delimiter = ',')]
    system_codes: Vec<String>,

    /// Milliseconds between polls while waiting for a card.
    #[arg(long, default_value_t = 500)]
    poll_interval_ms: u64,

    /// Console log detail: default = flow (info), `-v` = frames + HTTP (debug),
    /// `-vv` = raw bodies and every poll (trace). `RUST_LOG` overrides this.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

/// Options for the default one-shot mode (ignored by `serve`).
#[derive(Args, Debug)]
struct OnceArgs {
    /// Operation: `pay`, `topup`, or `balance`.
    #[arg(long, default_value = "pay")]
    op: String,

    /// Amount in yen (required for `pay`/`topup`; ignored for `balance`).
    #[arg(long)]
    amount: Option<i64>,

    /// Give up after this many seconds without an authenticated card
    /// (0 = wait indefinitely).
    #[arg(long, default_value_t = 0)]
    timeout_secs: u64,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run a local Web UI kiosk that owns the reader (touch UI in a browser).
    Serve(ServeArgs),
}

#[derive(Args, Debug)]
struct ServeArgs {
    /// Address to bind the local kiosk server to.
    #[arg(long, default_value = "127.0.0.1:8899")]
    bind: String,
}

/// Diagnostics go to **stderr** so stdout stays the operation's result (pipeable).
/// `RUST_LOG` wins; otherwise `-v`/`-vv` raise our crate's level.
fn init_logging(verbose: u8) {
    let level = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("melon_terminal={level},warn")));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_logging(cli.conn.verbose);
    let http = http_client();

    // The usable systems come from the server (those it holds keys for).
    // `--system-code` overrides that, e.g. for testing against a subset.
    let system_codes = if cli.conn.system_codes.is_empty() {
        fetch_system_codes(&http, &cli.conn.server, &cli.conn.api_key)
            .context("fetching usable system codes from the server")?
    } else {
        let codes = cli
            .conn
            .system_codes
            .iter()
            .map(|s| parse_u16(s))
            .collect::<Result<Vec<_>>>()
            .context("--system-code")?;
        tracing::info!(
            system_codes = %melon_terminal::fmt_codes(&codes),
            "usable systems overridden by --system-code (server list not fetched)"
        );
        codes
    };

    let cfg = Config {
        server: cli.conn.server,
        api_key: cli.conn.api_key,
        system_codes,
        poll_interval: Duration::from_millis(cli.conn.poll_interval_ms),
    };

    match cli.command {
        Some(Command::Serve(s)) => melon_terminal::serve::run(cfg, &s.bind),
        None => run_once(&cfg, &cli.once, &http),
    }
}

/// One-shot CLI: wait for a card, resolve its system, authenticate, run the
/// operation, print it.
fn run_once(cfg: &Config, once: &OnceArgs, http: &reqwest::blocking::Client) -> Result<()> {
    let op = Op::parse(&once.op)?;

    // The flow itself (reader, polling, Request System Code, selection, relay steps,
    // HTTP calls) is logged to stderr by the library — see `init_logging`.
    let mut reader = open_reader_auto()?;
    let target = make_target()?;

    // Phase 1 — wildcard-poll until a card is present (the only retry we do).
    let deadline =
        (once.timeout_secs > 0).then(|| Instant::now() + Duration::from_secs(once.timeout_secs));
    let poll = wait_for_card(&mut reader, &target, cfg.poll_interval, || match deadline {
        Some(d) if Instant::now() >= d => Some(WaitAbort::Timeout),
        _ => None,
    })
    .map_err(|_| anyhow!("no card presented within {}s", once.timeout_secs))?;

    // Phase 2 — a SINGLE attempt: ask the card which systems it has, pick the first
    // the server can authenticate, re-poll it (each system has its own IDm), and
    // authenticate. Any failure aborts — we never re-read the card or re-send.
    let card = resolve_card(&mut reader, &target, &poll, &cfg.system_codes)?;
    let (session_id, _idi) = authenticate(
        http,
        cfg,
        &mut reader,
        &target,
        card.system_code,
        &card.poll,
    )?;

    let result = run_operation(http, cfg, &session_id, op, once.amount)?;
    match op {
        Op::Balance => print_balance(&result),
        _ => println!("{}", serde_json::to_string_pretty(&result)?),
    }
    Ok(())
}

/// Print an authenticated card's balance and its per-expiry breakdown.
fn print_balance(bal: &Value) {
    println!(
        "balance: ¥{}  (system 0x{:04X}, idi {})",
        bal["total"].as_i64().unwrap_or(0),
        bal["system_code"].as_u64().unwrap_or(0),
        bal["idi"].as_str().unwrap_or("?"),
    );
    if let Some(buckets) = bal["buckets"].as_array() {
        for b in buckets {
            println!(
                "  ¥{}  expires {}",
                b["remaining"].as_i64().unwrap_or(0),
                b["expires_at"].as_str().unwrap_or("?"),
            );
        }
    }
}
