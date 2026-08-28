//! ventctl — headless Ventrilo 3 smoke-test client (parity with VentMac).
//!
//!   ventctl -h host[:port] -u username [-p password] [-c channel_id] [--stay] [--talk]
//!   ventctl devices
//!
//! `V3_DEBUG=1` enables libventrilo3 debug output.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::io::{self, BufRead};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
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
    let (host, port) = split_host(hostport);
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

    let _ = ctrlc::set_handler(|| {
        println!("\ndisconnecting…");
        Client::disconnect();
    });

    println!("connecting to {host}:{port} as {username}…");
    let rx = Client::connect(&host, port, username, &cli.password, "");

    for ev in rx {
        while toggle_rx.try_recv().is_ok() {
            toggle_talk(&transmitting, &mut capture);
        }
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
            CoreEvent::UserRemoved(id) => {
                if let Some(u) = roster.users.get(id) {
                    println!("{} logged out", u.name);
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
    Ok(())
}

fn split_host(s: &str) -> (String, u16) {
    if let Some((h, p)) = s.rsplit_once(':') {
        if let Ok(port) = p.parse() {
            return (h.to_string(), port);
        }
    }
    (s.to_string(), 3784)
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
