//! Headless smoke test for a server connection.
//!
//! `sqltool-smoke check` runs against a real server and prints a report that
//! contains no passwords and no table data, so it is safe to paste into a bug
//! report. `sqltool-smoke demo` runs the same checks against built-in fake
//! servers, for when no real server is reachable.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use sqltool_core::{CaSource, ConnectProfile, Flavor, Session, TlsMode};
use sqltool_fakeserver::{generate_identity, Config, FakeServer};

#[derive(Parser)]
#[command(name = "sqltool-smoke", about = "Connection smoke test for MySQL and MariaDB")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check a real server. The password is read from SQLTOOL_PASSWORD or prompted for.
    Check(CheckArgs),
    /// Run the checks against built-in fake MySQL Enterprise 8.4 and 5.7 servers.
    Demo {
        /// Rows to stream in the throughput check.
        #[arg(long, default_value_t = 1_000_000)]
        rows: u64,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum TlsArg {
    Disabled,
    Required,
    VerifyCa,
    VerifyIdentity,
}

#[derive(Args)]
struct CheckArgs {
    #[arg(long)]
    host: String,
    #[arg(long, default_value_t = 3306)]
    port: u16,
    #[arg(long)]
    user: String,
    #[arg(long)]
    database: Option<String>,
    #[arg(long, value_enum, default_value = "verify-identity")]
    tls: TlsArg,
    /// PEM file with the CA that signed the server certificate.
    #[arg(long)]
    ca: Option<PathBuf>,
    /// Allow the mysql_clear_password plugin (LDAP and PAM accounts).
    #[arg(long)]
    cleartext: bool,
    /// Allow cleartext auth without verified TLS. Only for links secured another way.
    #[arg(long)]
    accept_cleartext_risk: bool,
    /// A SELECT to stream for the throughput check. Only row counts and timings are printed.
    #[arg(long)]
    stream_query: Option<String>,
}

struct Checks {
    metadata: bool,
    stream_query: Option<String>,
}

fn line(ok: bool, label: &str, detail: impl AsRef<str>) {
    println!("  [{}] {:<18} {}", if ok { " ok " } else { "FAIL" }, label, detail.as_ref());
}

/// Runs every check and returns whether all of them passed.
async fn run_checks(profile: &ConnectProfile, checks: &Checks) -> bool {
    let started = Instant::now();
    let mut session = match Session::connect(profile).await {
        Ok(s) => s,
        Err(e) => {
            line(false, "connect", e.to_string());
            return false;
        }
    };
    line(true, "connect", format!("{} ms", started.elapsed().as_millis()));

    let info = session.info().clone();
    let flavor = match info.version.flavor {
        Flavor::MySql if info.version.enterprise => "MySQL Enterprise",
        Flavor::MySql => "MySQL",
        Flavor::MariaDb => "MariaDB",
        Flavor::Percona => "Percona Server",
    };
    line(
        true,
        "server",
        format!("{flavor} {}.{}.{}  (reported as \"{}\")", info.version.major, info.version.minor, info.version.patch, info.version.raw),
    );
    line(true, "logged in as", &info.current_user);
    match (&info.tls_version, profile.tls) {
        (Some(v), _) => {
            let cipher = info.tls_cipher.as_deref().unwrap_or("cipher not reported");
            line(true, "encryption", format!("{v}, {cipher}"));
        }
        (None, TlsMode::Disabled) => line(true, "encryption", "none (TLS disabled in profile)"),
        (None, _) => line(false, "encryption", "profile asked for TLS but the server reports none"),
    }
    let mut all_ok = info.tls_version.is_some() || profile.tls == TlsMode::Disabled;

    match session.query("SELECT 1").await {
        Ok(r) if r.rows == vec![vec![Some("1".to_string())]] => line(true, "query", "SELECT 1 round trip"),
        Ok(r) => {
            all_ok = false;
            line(false, "query", format!("unexpected result: {:?}", r.rows));
        }
        Err(e) => {
            all_ok = false;
            line(false, "query", e.to_string());
        }
    }

    if checks.metadata {
        match session.query("SELECT COUNT(*) FROM information_schema.schemata").await {
            Ok(r) => {
                let n = r.rows.first().and_then(|row| row[0].clone()).unwrap_or_default();
                line(true, "metadata", format!("{n} schemas visible"));
            }
            Err(e) => {
                all_ok = false;
                line(false, "metadata", e.to_string());
            }
        }
    }

    // Start a long query, interrupt it from a second connection, and confirm
    // it returns promptly and the session still works.
    let canceller = session.canceller();
    let cancel = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        canceller.cancel().await
    });
    let started = Instant::now();
    let slept = session.query("SELECT SLEEP(20)").await;
    let elapsed = started.elapsed();
    let cancel_result = cancel.await.expect("cancel task");
    let usable = session.query("SELECT 1").await.is_ok();
    match (cancel_result, slept.is_ok() || elapsed < Duration::from_secs(10), usable) {
        (Ok(()), true, true) if elapsed < Duration::from_secs(10) => {
            line(true, "cancel", format!("interrupted after {} ms", elapsed.as_millis()))
        }
        (Err(e), _, _) => {
            all_ok = false;
            line(false, "cancel", format!("could not send KILL QUERY: {e}"));
        }
        _ => {
            all_ok = false;
            line(false, "cancel", format!("query ran {} ms; session usable: {usable}", elapsed.as_millis()));
        }
    }

    if let Some(sql) = &checks.stream_query {
        match session.stream(sql, |_columns| {}, |_row| {}).await {
            Ok(s) => {
                let secs = s.total.as_secs_f64().max(1e-9);
                line(
                    true,
                    "streaming",
                    format!(
                        "{} rows, {:.1} MB in {:.2} s  ({:.0} rows/s, first row after {} ms)",
                        s.rows,
                        s.bytes as f64 / 1_048_576.0,
                        secs,
                        s.rows as f64 / secs,
                        s.time_to_first_row.map(|d| d.as_millis()).unwrap_or(0),
                    ),
                );
            }
            Err(e) => {
                all_ok = false;
                line(false, "streaming", e.to_string());
            }
        }
    }

    let _ = session.close().await;
    all_ok
}

async fn check(args: CheckArgs) -> Result<bool> {
    let password = match std::env::var("SQLTOOL_PASSWORD") {
        Ok(p) => p,
        Err(_) => rpassword::prompt_password(format!("Password for {}@{}: ", args.user, args.host)).context("reading password")?,
    };
    let mut profile = ConnectProfile::new(args.host.clone(), args.port, args.user.clone(), password.into());
    profile.database = args.database;
    profile.tls = match args.tls {
        TlsArg::Disabled => TlsMode::Disabled,
        TlsArg::Required => TlsMode::Required,
        TlsArg::VerifyCa => TlsMode::VerifyCa,
        TlsArg::VerifyIdentity => TlsMode::VerifyIdentity,
    };
    profile.ca = args.ca.map(CaSource::File);
    profile.allow_cleartext_auth = args.cleartext;
    profile.accept_cleartext_on_unverified_transport = args.accept_cleartext_risk;

    println!("{}:{} as {}", args.host, args.port, args.user);
    Ok(run_checks(&profile, &Checks { metadata: true, stream_query: args.stream_query }).await)
}

async fn demo(rows: u64) -> Result<bool> {
    const PASSWORD: &str = "demo-password-not-a-real-secret";
    let mut all_ok = true;

    let presets: [(&str, fn(&str, Option<sqltool_fakeserver::TlsIdentity>) -> Config); 2] = [
        ("fake MySQL Enterprise 8.4.3, LDAP account", Config::mysql_8_4_enterprise_ldap),
        ("fake MySQL Enterprise 5.7.37, LDAP account, TLS 1.2 only", Config::mysql_5_7_enterprise_ldap),
    ];
    for (title, preset) in presets {
        let identity = generate_identity(&["127.0.0.1"]);
        let server = FakeServer::start(preset(PASSWORD, Some(identity.clone()))).await?;
        let mut profile = ConnectProfile::new("127.0.0.1", server.port(), "demo", PASSWORD.to_string().into());
        profile.ca = Some(CaSource::Pem(identity.ca_pem.clone().into_bytes()));
        profile.allow_cleartext_auth = true;

        println!("{title}");
        all_ok &= run_checks(&profile, &Checks { metadata: false, stream_query: Some(format!("SELECT fake_rows({rows}, 10)")) }).await;
        let seen = server.observed();
        let delivered = seen.cleartext_password.as_deref() == Some(PASSWORD) && seen.cleartext_over_tls == Some(true);
        all_ok &= delivered;
        line(delivered, "cleartext auth", "server received the password, inside TLS");

        // Same server, cleartext not enabled in the profile: must refuse.
        profile.allow_cleartext_auth = false;
        let before = server.observed().logins_ok;
        let refused = match Session::connect(&profile).await {
            Err(sqltool_core::Error::CleartextNotEnabled) => server.observed().logins_ok == before,
            _ => false,
        };
        all_ok &= refused;
        line(refused, "refusal", "with cleartext off, the login is refused and nothing is sent");
        println!();
    }
    Ok(all_ok)
}

#[tokio::main]
async fn main() -> Result<()> {
    let ok = match Cli::parse().command {
        Command::Check(args) => check(args).await?,
        Command::Demo { rows } => demo(rows).await?,
    };
    println!("{}", if ok { "All checks passed." } else { "Some checks FAILED." });
    std::process::exit(if ok { 0 } else { 1 });
}
