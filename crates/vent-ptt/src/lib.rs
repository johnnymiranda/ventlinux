//! Global PTT from `/dev/input/event*` without grabbing devices.
//!
//! Needs membership in the `input` group. Does not steal keys from games.

use evdev::{Device, EventType, KeyCode};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingKind {
    Key,
    Mouse,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub kind: BindingKind,
    /// evdev key/button code (KEY_LEFTCTRL = 29, BTN_SIDE = 275, BTN_EXTRA = 276).
    pub code: u16,
    pub display: String,
}

impl Default for Binding {
    fn default() -> Self {
        Self {
            kind: BindingKind::Key,
            code: KeyCode::KEY_LEFTCTRL.0,
            display: "Left Ctrl".into(),
        }
    }
}

impl Binding {
    pub fn matches(&self, code: u16) -> bool {
        self.code == code
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PttEvent {
    Down,
    Up,
}

pub struct Ptt {
    binding: Arc<Mutex<Binding>>,
    capturing: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl Ptt {
    pub fn start(binding: Binding) -> (Self, Receiver<PttEvent>, Receiver<Binding>) {
        let binding = Arc::new(Mutex::new(binding));
        let capturing = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (ev_tx, ev_rx) = mpsc::channel();
        let (bind_tx, bind_rx) = mpsc::channel();

        let b = binding.clone();
        let cap = capturing.clone();
        let stop_f = stop.clone();
        thread::spawn(move || watch_loop(b, cap, stop_f, ev_tx, bind_tx));

        (
            Self {
                binding,
                capturing,
                stop,
            },
            ev_rx,
            bind_rx,
        )
    }

    pub fn binding(&self) -> Binding {
        self.binding.lock().unwrap().clone()
    }

    pub fn set_binding(&self, binding: Binding) {
        *self.binding.lock().unwrap() = binding;
    }

    pub fn begin_capture(&self) {
        self.capturing.store(true, Ordering::SeqCst);
    }

    pub fn end_capture(&self) {
        self.capturing.store(false, Ordering::SeqCst);
    }

    pub fn is_capturing(&self) -> bool {
        self.capturing.load(Ordering::SeqCst)
    }
}

impl Drop for Ptt {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn watch_loop(
    binding: Arc<Mutex<Binding>>,
    capturing: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    ev_tx: mpsc::Sender<PttEvent>,
    bind_tx: mpsc::Sender<Binding>,
) {
    let mut devices = open_devices();
    let mut last_rescan = std::time::Instant::now();
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if last_rescan.elapsed() > Duration::from_secs(3) {
            devices = open_devices();
            last_rescan = std::time::Instant::now();
        }
        let mut any = false;
        for dev in &mut devices {
            match dev.fetch_events() {
                Ok(iter) => {
                    for ev in iter {
                        any = true;
                        handle(ev, &binding, &capturing, &ev_tx, &bind_tx);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => {}
            }
        }
        if !any {
            thread::sleep(Duration::from_millis(5));
        }
    }
}

fn open_devices() -> Vec<Device> {
    let mut out = Vec::new();
    for (_, d) in evdev::enumerate() {
        let _ = d.set_nonblocking(true);
        out.push(d);
    }
    out
}

fn handle(
    ev: evdev::InputEvent,
    binding: &Mutex<Binding>,
    capturing: &AtomicBool,
    ev_tx: &mpsc::Sender<PttEvent>,
    bind_tx: &mpsc::Sender<Binding>,
) {
    if ev.event_type() != EventType::KEY {
        return;
    }
    let code = ev.code();
    let value = ev.value();
    if value == 2 {
        return; // repeat
    }
    let down = value == 1;

    if capturing.load(Ordering::SeqCst) && down {
        let new = binding_from_code(code);
        *binding.lock().unwrap() = new.clone();
        capturing.store(false, Ordering::SeqCst);
        let _ = bind_tx.send(new);
        return;
    }

    let b = binding.lock().unwrap().clone();
    if b.matches(code) {
        let _ = ev_tx.send(if down { PttEvent::Down } else { PttEvent::Up });
    }
}

fn binding_from_code(code: u16) -> Binding {
    let (kind, display) = match code {
        c if c == KeyCode::BTN_LEFT.0 => (BindingKind::Mouse, "Mouse 1".into()),
        c if c == KeyCode::BTN_RIGHT.0 => (BindingKind::Mouse, "Mouse 2".into()),
        c if c == KeyCode::BTN_MIDDLE.0 => (BindingKind::Mouse, "Mouse 3".into()),
        c if c == KeyCode::BTN_SIDE.0 => (BindingKind::Mouse, "Mouse 4".into()),
        c if c == KeyCode::BTN_EXTRA.0 => (BindingKind::Mouse, "Mouse 5".into()),
        _ => (BindingKind::Key, key_name(code)),
    };
    Binding { kind, code, display }
}

fn key_name(code: u16) -> String {
    let s = format!("{:?}", KeyCode(code));
    s.strip_prefix("KeyCode(")
        .and_then(|t| t.strip_suffix(')'))
        .unwrap_or(&s)
        .trim_matches('"')
        .to_string()
}
