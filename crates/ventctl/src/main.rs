//! ventctl — headless Ventrilo 3 smoke-test client (parity with VentMac).
//!
//!   ventctl -h host[:port] -u username [-p password] [-c channel_id] [--stay] [--talk]
//!   ventctl devices
//!
//! `V3_DEBUG=1` enables libventrilo3 debug output.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::io::{self, BufRead};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use vent_audio::{Capture, Player};
use vent_core::{Client, CoreEvent, Roster, TreeNode};

#[derive(Parser)]
#[command(name = "ventctl", about = "Headless Ventrilo 3 client")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// host[:port]
    #[arg(short = 's', long = "host")]
    host: Option<String>,
    #[arg(short, long)]
    username: Option<String>,
    #[arg(short, long, default_value = "")]
    password: String,
    #[arg(short, long)]
    channel: Option<u16>,
    /// Remain connected and play incoming voice
    #[arg(long)]
    stay: bool,
    /// With --stay: press Enter to toggle transmit
    #[arg(long)]
    talk: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// List capture/playback devices
    Devices,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if matches!(cli.cmd, Some(Cmd::Devices)) {
        let (ins, outs) = vent_audio::list_devices();
        println!("Inputs:");
        for d in ins {
            println!("  • {}", d.name);
        }
        println!("Outputs:");
        for d in outs {
            println!("  • {}", d.name);
        }
        return Ok(());
    }

    let hostport = cli.host.as_deref().unwrap_or("");
    let username = cli.username.as_deref().unwrap_or("");
    if hostport.is_empty() || username.is_empty() {
        eprintln!(
            "usage: ventctl -s host[:port] -u username [-p password] [-c channel_id] [--stay] [--talk]"
        );
        std::process::exit(1);
    }
    let (host, port) = split_host(hostport)?;
    let stay = cli.stay || cli.talk;
    let talk = cli.talk;

    let player = if stay {
        Some(Player::start(None)?)
    } else {
        None
    };
    if let Some(p) = &player {
        let h = p.handle();
        Client::set_audio_sink(move |uid, rate, ch, pcm| h.play(uid, rate, ch, pcm));
    }

    let mut roster = Roster::default();
    let transmitting = Arc::new(AtomicBool::new(false));
    let mut capture: Option<Capture> = None;
    let (toggle_tx, toggle_rx) = std::sync::mpsc::channel::<()>();

    if talk {
        let toggle_tx = toggle_tx.clone();
        thread::spawn(move || {
            let stdin = io::stdin();
            for _ in stdin.lock().lines() {
                let _ = toggle_tx.send(());
            }
        });
    }

    // v3_logout only does anything once we are logged in, so before that a
    // graceful disconnect would silently swallow the signal and leave a hung
    // connect unkillable. A second Ctrl+C always exits.
    let interrupted = Arc::new(AtomicBool::new(false));
    let sig = interrupted.clone();
    let _ = ctrlc::set_handler(move || {
        if !Client::is_logged_in() || sig.swap(true, Ordering::SeqCst) {
            std::process::exit(130);
        }
        println!("\ndisconnecting…");
        Client::disconnect();
    });

    println!("connecting to {host}:{port} as {username}…");
    let rx = Client::connect(&host, port, username, &cli.password, "");

    loop {
        while toggle_rx.try_recv().is_ok() {
            if Client::is_logged_in() {
                toggle_talk(&transmitting, &mut capture);
            }
        }
        let ev = match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(ev) => ev,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let removed_user = match &ev {
            CoreEvent::UserRemoved(id) => roster.users.get(id).map(|u| u.name.clone()),
            _ => None,
        };
        roster.apply(&ev);
        match &ev {
            CoreEvent::Status { percent, message } => println!("[{percent}%] {message}"),
            CoreEvent::LoginFailed(msg) => {
                eprintln!("LOGIN FAILED: {msg}");
                std::process::exit(2);
            }
            CoreEvent::ErrorMessage {
                message,
                disconnected,
            } => {
                eprintln!(
                    "SERVER ERROR: {message}{}",
                    if *disconnected { " (disconnected)" } else { "" }
                );
                if *disconnected {
                    std::process::exit(2);
                }
            }
            CoreEvent::LoginCompleted => {
                println!("login complete — user id {}", Client::own_user_id());
                if let Some(codec) = Client::codec_for_channel(0) {
                    let warn = if codec.is_supported() {
                        ""
                    } else {
                        "  ⚠️ NOT SUPPORTED"
                    };
                    println!(
                        "server default codec: {} @ {} Hz{warn}",
                        codec.name, codec.rate
                    );
                }
                print_tree(&roster);
                if let Some(ch) = cli.channel {
                    println!("joining channel {}…", roster.channel_name(ch));
                    Client::join_channel(ch, "");
                }
                if !stay {
                    Client::disconnect();
                } else if talk {
                    println!("press Enter to toggle transmit");
                }
            }
            CoreEvent::ChannelPasswordRejected(id) => {
                println!("bad password for channel {}", roster.channel_name(*id));
            }
            CoreEvent::UserUpserted(u) if Client::is_logged_in() && !u.name.is_empty() => {
                println!("{} → {}", u.name, roster.channel_name(u.channel_id));
            }
            CoreEvent::UserRemoved(_) => {
                if let Some(name) = removed_user.as_deref() {
                    println!("{name} logged out");
                }
            }
            CoreEvent::MovedToChannel(id) => {
                println!("you are now in {}", roster.channel_name(*id));
            }
            CoreEvent::TalkStarted { user_id, .. } if stay => {
                let name = roster
                    .users
                    .get(user_id)
                    .map(|u| u.name.as_str())
                    .unwrap_or("?");
                println!("🎙 {name} talking");
            }
            CoreEvent::TalkEnded { user_id } if stay => {
                let name = roster
                    .users
                    .get(user_id)
                    .map(|u| u.name.as_str())
                    .unwrap_or("?");
                println!("   {name} stopped");
            }
            CoreEvent::Motd(m) => {
                let t = m.trim();
                if !t.is_empty() {
                    println!("MOTD: {}", t.chars().take(500).collect::<String>());
                }
            }
            CoreEvent::Disconnected => println!("disconnected"),
            CoreEvent::ChatMessage { user_id, message } if stay => {
                let name = if *user_id == 0 {
                    "[server]".into()
                } else {
                    roster
                        .users
                        .get(user_id)
                        .map(|u| u.name.clone())
                        .unwrap_or_else(|| format!("#{user_id}"))
                };
                println!("💬 {name}: {message}");
            }
            CoreEvent::Paged { from_user } => {
                let name = roster
                    .users
                    .get(from_user)
                    .map(|u| u.name.as_str())
                    .unwrap_or("?");
                println!("📟 page from {name}");
            }
            _ => {}
        }
    }
    if transmitting.load(Ordering::SeqCst) {
        capture = None;
        Client::stop_transmit();
    }
    Client::clear_audio_sink();
    drop(capture);
    Ok(())
}

fn split_host(s: &str) -> Result<(String, u16)> {
    if s.is_empty() {
        bail!("host cannot be empty");
    }
    let colon_count = s.bytes().filter(|b| *b == b':').count();
    if colon_count > 1 || s.starts_with('[') {
        bail!("IPv6 server addresses are not supported by libventrilo3");
    }
    if let Some((host, port)) = s.split_once(':') {
        if host.is_empty() {
            bail!("host cannot be empty");
        }
        let port = port
            .parse::<u16>()
            .with_context(|| format!("invalid server port '{port}'"))?;
        if port == 0 {
            bail!("server port must be between 1 and 65535");
        }
        return Ok((host.to_string(), port));
    }
    Ok((s.to_string(), 3784))
}

fn print_tree(roster: &Roster) {
    println!("\n── Channel tree ──────────────────────────");
    for (depth, node) in roster.flattened_tree() {
        let indent = "   ".repeat(depth);
        match node {
            TreeNode::Channel(ch) => {
                let lock = if ch.password_protected { " 🔒" } else { "" };
                let codec = Client::codec_for_channel(ch.id)
                    .map(|c| format!(" [{}]", c.name))
                    .unwrap_or_default();
                println!("{indent}▸ {} (id {}){lock}{codec}", ch.name, ch.id);
            }
            TreeNode::User(u) => println!("{indent}• {}", u.name),
        }
    }
    println!("──────────────────────────────────────────");
}

fn toggle_talk(transmitting: &AtomicBool, capture: &mut Option<Capture>) {
    if transmitting.swap(true, Ordering::SeqCst) {
        transmitting.store(false, Ordering::SeqCst);
        *capture = None;
        Client::stop_transmit();
        println!(">>> stopped transmitting — press Enter to talk");
    } else {
        Client::start_transmit();
        match Capture::start(None, |pcm, rate| Client::send_pcm(pcm, rate, false)) {
            Ok(c) => {
                *capture = Some(c);
                println!(">>> TRANSMITTING — press Enter to stop");
            }
            Err(e) => {
                Client::stop_transmit();
                transmitting.store(false, Ordering::SeqCst);
                eprintln!(">>> {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_host_uses_default_port() {
        assert_eq!(
            split_host("vent.example.com").unwrap(),
            ("vent.example.com".into(), 3784)
        );
    }

    #[test]
    fn split_host_accepts_explicit_port() {
        assert_eq!(
            split_host("127.0.0.1:4000").unwrap(),
            ("127.0.0.1".into(), 4000)
        );
    }

    #[test]
    fn split_host_rejects_invalid_ports() {
        assert!(split_host("vent.example.com:nope").is_err());
        assert!(split_host("vent.example.com:0").is_err());
    }

    #[test]
    fn split_host_rejects_unsupported_ipv6() {
        assert!(split_host("2001:db8::1").is_err());
        assert!(split_host("[::1]:3784").is_err());
    }
}
