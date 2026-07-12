//! `melon-terminal` — the merchant terminal binary.
//!
//! Two modes share the same reader/relay logic (in the `melon_terminal`
//! library). The mode is chosen by what you pass:
//!
//! * **Web UI kiosk (default)** — run with no operation flags and it launches a
//!   long-running local touch UI at `http://<bind>/` that owns the reader.
//! * **one-shot CLI** — pass an operation (`--op` and/or `--amount`) and it
//!   waits for a card, authenticates, runs that one operation, prints the
//!   result, and exits. Handy for scripting and hardware bring-up.
//!
//! Requires a physical reader, so it is exercised against hardware, not in CI.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use clap::{Args, Parser};
use serde_json::Value;
use tracing_subscriber::EnvFilter;

use melon_terminal::{
    Config, Op, WaitAbort, authenticate, fetch_system_codes, http_client, make_target,
    open_reader_auto, resolve_card, run_operation, wait_for_card,
};

#[derive(Parser, Debug)]
#[command(
    name = "melon-terminal",
    version,
    about = "加盟店端末。引数なしで Web UI キオスクを起動し、--op / --amount を指定すると CLI 一発実行になります。"
)]
struct Cli {
    #[command(flatten)]
    conn: ConnArgs,

    #[command(flatten)]
    once: OnceArgs,

    /// Web UI kiosk bind address (used when no one-shot operation is requested).
    #[arg(long, default_value = "127.0.0.1:8899")]
    bind: String,

    /// Do NOT open the Web UI in the default browser on kiosk startup
    /// (e.g. for a headless/remote host). Ignored in one-shot mode.
    #[arg(long)]
    no_open: bool,
}

/// Connection/reader options shared by every mode.
#[derive(Args, Debug)]
struct ConnArgs {
    /// Base URL of the melon server.
    #[arg(
        long,
        env = "MELON_SERVER",
        default_value = "https://melon.unknowntech.jp"
    )]
    server: String,

    /// Merchant API key (bearer secret). Required for a one-shot operation;
    /// optional for the Web UI kiosk (which can be configured from its settings
    /// screen instead).
    #[arg(long, env = "MELON_API_KEY")]
    api_key: Option<String>,

    /// Milliseconds between polls while waiting for a card.
    #[arg(long, default_value_t = 500)]
    poll_interval_ms: u64,

    /// Console log detail: default = flow (info), `-v` = frames + HTTP (debug),
    /// `-vv` = raw bodies and every poll (trace). `RUST_LOG` overrides this.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

/// One-shot CLI options. Supplying `--op` or `--amount` selects one-shot mode;
/// with neither, the terminal launches the Web UI kiosk instead.
#[derive(Args, Debug)]
struct OnceArgs {
    /// Operation: `pay`, `topup`, or `balance` (defaults to `pay` when only
    /// `--amount` is given). Passing this selects one-shot CLI mode.
    #[arg(long)]
    op: Option<String>,

    /// Amount in yen (required for `pay`/`topup`; ignored for `balance`).
    /// Passing this selects one-shot CLI mode.
    #[arg(long)]
    amount: Option<i64>,

    /// Give up after this many seconds without an authenticated card
    /// (0 = wait indefinitely).
    #[arg(long, default_value_t = 0)]
    timeout_secs: u64,
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
    let poll_interval = Duration::from_millis(cli.conn.poll_interval_ms);

    // A requested operation (`--op` and/or `--amount`) means one-shot CLI;
    // otherwise the default is the Web UI kiosk. Connection-only flags
    // (`--api-key`, `--server`, `-v`, `--bind`) do NOT force one-shot.
    let one_shot = cli.once.op.is_some() || cli.once.amount.is_some();

    if one_shot {
        let api_key = cli.conn.api_key.ok_or_else(|| {
            anyhow!("--api-key (or MELON_API_KEY) is required for a one-shot operation")
        })?;
        // The usable systems (and their priority order) come from the server —
        // those it holds keys for. See `resolve_card` for how one is chosen.
        let system_codes = fetch_system_codes(&http, &cli.conn.server, &api_key)
            .context("fetching usable system codes from the server")?;
        let cfg = Config {
            server: cli.conn.server,
            api_key,
            system_codes,
            poll_interval,
        };
        run_once(&cfg, &cli.once, &http)
    } else {
        // The kiosk may start without a key (it reuses a saved one or is configured
        // from its screen). It opens the UI in the default browser unless suppressed.
        melon_terminal::serve::run(
            cli.conn.server,
            cli.conn.api_key,
            poll_interval,
            &cli.bind,
            !cli.no_open,
        )
    }
}

/// One-shot CLI: wait for a card, resolve its system, authenticate, run the
/// operation, print it.
fn run_once(cfg: &Config, once: &OnceArgs, http: &reqwest::blocking::Client) -> Result<()> {
    // `--amount` alone (no `--op`) means the common case: a payment.
    let op = Op::parse(once.op.as_deref().unwrap_or("pay"))?;

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
