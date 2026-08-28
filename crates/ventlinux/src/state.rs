use crate::config::{self, AppConfig, SavedServer};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use vent_audio::{Capture, Player};
use vent_core::{Client, CoreEvent, Roster, User, VoxAction, VoxGate};
use vent_ptt::{Binding, Ptt, PttEvent};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TransmitMode {
    Ptt,
    Vox,
}

pub struct Session {
    pub status: Status,
    pub roster: Roster,
    pub own_channel_id: u16,
    pub own_user_id: u16,
    pub last_error: Option<String>,
    pub connect_status: String,
    pub codec_label: String,
    pub ping: Option<u16>,
    pub transmitting: bool,
    pub sound_muted: bool,
    pub mic_muted: bool,
    pub transmit_mode: TransmitMode,
    pub vox_level: f32,
    pub chat_open: bool,
    pub chat_log: Vec<String>,
    pub motd: Option<String>,
    pub server_name: String,
    pub reconnect_attempt: u32,
    pub servers: Vec<SavedServer>,
    pub selected_server_id: Option<String>,
    pub config: AppConfig,
    pub password_prompt: Option<(u16, String)>,

    pub events: Option<Receiver<CoreEvent>>,
    pub ptt: Option<Ptt>,
    pub ptt_events: Option<Receiver<PttEvent>>,
    pub ptt_binds: Option<Receiver<Binding>>,
    pub player: Option<Player>,
    pub capture: Option<Capture>,
    vox: VoxGate,
    vox_open: bool,
    want_disconnect: bool,
    suppress_reconnect: bool,
    conn: Option<(String, u16, String, String)>,
    last_channel: u16,
    next_retry: Option<Instant>,
}

impl Session {
    pub fn new() -> Self {
        let mut config = config::load_config();
        // First builds defaulted to F13; most keyboards don't have it. Run this
        // once only, so a user who deliberately picks F13 keeps it.
        if !config.ptt_migrated {
            if config.ptt.code == 183 || config.ptt.display == "F13" {
                config.ptt = vent_ptt::Binding::default();
            }
            config.ptt_migrated = true;
            let _ = config::save_config(&config);
        }
        let mode = if config.transmit_mode == "vox" {
            TransmitMode::Vox
        } else {
            TransmitMode::Ptt
        };
        let mut vox = VoxGate::default();
        vox.config.open_threshold_dbfs = config.vox_sensitivity;
        vox.config.close_threshold_dbfs = config.vox_sensitivity - 10.0;
        let (ptt, ptt_events, ptt_binds) = Ptt::start(config.ptt.clone());
        let servers = config::load_servers();
        let selected_server_id = servers.first().map(|s| s.id.clone());
        Self {
            status: Status::Disconnected,
            roster: Roster::default(),
            own_channel_id: 0,
            own_user_id: 0,
            last_error: None,
            connect_status: String::new(),
            codec_label: String::new(),
            ping: None,
            transmitting: false,
            sound_muted: false,
            mic_muted: false,
            transmit_mode: mode,
            vox_level: -120.0,
            chat_open: false,
            chat_log: Vec::new(),
            motd: None,
            server_name: String::new(),
            reconnect_attempt: 0,
            servers,
            selected_server_id,
            config,
            password_prompt: None,
            events: None,
            ptt: Some(ptt),
            ptt_events: Some(ptt_events),
            ptt_binds: Some(ptt_binds),
            player: None,
            capture: None,
            vox,
            vox_open: false,
            want_disconnect: false,
            suppress_reconnect: false,
            conn: None,
            last_channel: 0,
            next_retry: None,
        }
    }

    pub fn persist(&self) {
        let _ = config::save_servers(&self.servers);
        let _ = config::save_config(&self.config);
    }

    pub fn selected_server(&self) -> Option<SavedServer> {
        if let Some(id) = &self.selected_server_id {
            if let Some(s) = self.servers.iter().find(|s| &s.id == id) {
                return Some(s.clone());
            }
        }
        if self.servers.len() == 1 {
            return self.servers.first().cloned();
        }
        None
    }

    pub fn select_server(&mut self, id: String) {
        self.selected_server_id = Some(id);
    }

    pub fn remove_selected(&mut self) -> bool {
        let Some(sel) = self.selected_server() else {
            self.last_error = Some("Select a server first.".into());
            return false;
        };
        self.servers.retain(|s| s.id != sel.id);
        self.selected_server_id = self.servers.first().map(|s| s.id.clone());
        self.last_error = None;
        self.persist();
        true
    }

    pub fn connect(&mut self, server: &SavedServer) {
        if self.status != Status::Disconnected {
            return;
        }
        let server_changed = self.conn.as_ref().is_some_and(|(host, port, user, _)| {
            host != &server.host || *port != server.port || user != &server.username
        });
        if server_changed {
            self.last_channel = 0;
        }
        self.conn = Some((
            server.host.clone(),
            server.port,
            server.username.clone(),
            server.password.clone(),
        ));
        self.want_disconnect = false;
        self.reconnect_attempt = 0;
        self.server_name = server.display_address();
        self.start_session();
    }

    fn start_session(&mut self) {
        let Some((host, port, user, pass)) = self.conn.clone() else {
            return;
        };
        self.status = if self.reconnect_attempt == 0 {
            Status::Connecting
        } else {
            Status::Reconnecting
        };
        self.last_error = None;
        self.connect_status.clear();
        self.roster = Roster::default();
        self.chat_log.clear();
        self.own_channel_id = 0;
        self.own_user_id = 0;
        self.codec_label.clear();
        self.motd = None;
        self.ping = None;

        match Player::start(nonempty(&self.config.output_device)) {
            Ok(p) => {
                p.set_muted(self.sound_muted);
                let h = p.handle();
                Client::set_audio_sink(move |uid, rate, ch, pcm| {
                    h.play(uid, rate, ch, pcm);
                });
                self.player = Some(p);
            }
            Err(e) => self.last_error = Some(format!("audio output: {e}")),
        }

        // Resolve the microphone now, in the background, so the first PTT press
        // is not delayed by ALSA device discovery.
        let input = self.config.input_device.clone();
        std::thread::spawn(move || vent_audio::prewarm_input(nonempty(&input)));

        self.events = Some(Client::connect(&host, port, &user, &pass, ""));
    }

    pub fn disconnect(&mut self) {
        self.want_disconnect = true;
        self.next_retry = None;
        self.stop_talk();
        self.stop_vox_capture();
        // Always ask the library to drop the session. A retry may already have
        // a login in flight, and leaving it running would bring the connection
        // back moments after the user asked to leave.
        Client::disconnect();
        if self.events.is_none() {
            // Nothing in flight (waiting between retries) — no stream end is
            // coming to move us out of Reconnecting.
            self.status = Status::Disconnected;
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut dirty = false;
        let binds: Vec<_> = self
            .ptt_binds
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for b in binds {
            self.config.ptt = b;
            self.persist();
            dirty = true;
        }
        let ptt_evs: Vec<_> = self
            .ptt_events
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for ev in ptt_evs {
            match ev {
                PttEvent::Down => self.ptt_down(),
                PttEvent::Up => self.ptt_up(),
            }
            if self.status == Status::Connected {
                dirty = true;
            }
        }
        let mut ended = false;
        let evs: Vec<_> = self
            .events
            .as_ref()
            .map(|rx| {
                let mut out = Vec::new();
                loop {
                    match rx.try_recv() {
                        Ok(ev) => out.push(ev),
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            ended = true;
                            break;
                        }
                    }
                }
                out
            })
            .unwrap_or_default();
        for ev in evs {
            if matches!(ev, CoreEvent::Disconnected) {
                ended = true;
            }
            self.handle(ev);
            dirty = true;
        }
        if ended {
            self.events = None;
            self.on_stream_end();
            dirty = true;
        }
        let vox_state = VOX.lock().ok().and_then(|g| {
            g.as_ref()
                .map(|(gate, open)| (gate.last_level_dbfs(), *open))
        });
        if let Some((level, open)) = vox_state {
            self.vox_level = level;
            self.vox_open = open;
            if self.transmit_mode == TransmitMode::Vox && self.transmitting != open {
                self.transmitting = open;
                dirty = true;
            }
        }
        if let Some(at) = self.next_retry {
            if Instant::now() >= at && self.status == Status::Reconnecting && !self.want_disconnect
            {
                self.next_retry = None;
                self.start_session();
                dirty = true;
            }
        }
        dirty
    }

    fn handle(&mut self, ev: CoreEvent) {
        self.roster.apply(&ev);
        match ev {
            CoreEvent::Status { percent, message } => {
                self.connect_status = format!("[{percent}%] {message}");
            }
            CoreEvent::LoginFailed(msg) => {
                self.last_error = Some(msg);
                // On a first attempt the status is already Connecting, so
                // on_stream_end gives up: bad host or credentials. During an
                // automatic reconnect a refused login usually just means the
                // server has not come back yet, so let the backoff keep going.
            }
            CoreEvent::ErrorMessage {
                message,
                disconnected,
            } => {
                self.last_error = Some(message);
                if disconnected {
                    // The server dropped us deliberately (kick, ban, full).
                    // Reconnecting would only hammer it.
                    self.suppress_reconnect = true;
                }
            }
            CoreEvent::LoginCompleted => {
                if self.want_disconnect {
                    // The user hit Disconnect while this login was in flight.
                    Client::disconnect();
                    return;
                }
                self.status = Status::Connected;
                self.own_user_id = Client::own_user_id();
                self.reconnect_attempt = 0;
                if let Some(c) = Client::codec_for_channel(0) {
                    self.codec_label = format!("{} @ {} Hz", c.name, c.rate);
                }
                if self.last_channel != 0 {
                    Client::join_channel(self.last_channel, "");
                }
                if self.chat_open {
                    Client::join_chat();
                }
                self.apply_vox_state();
            }
            CoreEvent::MovedToChannel(id) => {
                self.own_channel_id = id;
                self.last_channel = id;
            }
            CoreEvent::Ping(p) => self.ping = Some(p),
            CoreEvent::Motd(m) if !m.trim().is_empty() => self.motd = Some(m),
            CoreEvent::ChatMessage { user_id, message } => {
                let name = if user_id == 0 {
                    "server".into()
                } else {
                    self.roster
                        .users
                        .get(&user_id)
                        .map(|u| u.name.clone())
                        .unwrap_or_else(|| format!("#{user_id}"))
                };
                self.chat_log.push(format!("{name}: {message}"));
                if self.chat_log.len() > 400 {
                    self.chat_log.remove(0);
                }
            }
            CoreEvent::Disconnected => {}
            CoreEvent::ChannelPasswordRejected(id) => {
                let name = self.roster.channel_name(id);
                self.password_prompt = Some((id, name));
            }
            _ => {}
        }
    }

    fn on_stream_end(&mut self) {
        self.stop_talk();
        self.stop_vox_capture();
        if let Some(p) = self.player.take() {
            p.shutdown();
        }
        Client::clear_audio_sink();
        self.ping = None;
        let suppressed = std::mem::take(&mut self.suppress_reconnect);
        if suppressed
            || self.want_disconnect
            || self.conn.is_none()
            || self.status == Status::Connecting
        {
            self.status = Status::Disconnected;
            return;
        }
        if self.reconnect_attempt >= 20 {
            self.last_error = Some("Gave up reconnecting after 20 attempts.".into());
            self.status = Status::Disconnected;
            return;
        }
        self.status = Status::Reconnecting;
        self.reconnect_attempt += 1;
        let delay = (2u64.pow(self.reconnect_attempt.min(4))).min(30);
        self.next_retry = Some(Instant::now() + Duration::from_secs(delay));
    }

    pub fn join(&mut self, id: u16, password_protected: bool) {
        if password_protected {
            let name = self.roster.channel_name(id);
            self.password_prompt = Some((id, name));
        } else {
            Client::join_channel(id, "");
        }
    }

    pub fn join_with_password(&mut self, id: u16, password: &str) {
        Client::join_channel(id, password);
        self.password_prompt = None;
    }

    pub fn send_chat(&mut self, text: &str) {
        let t = text.trim();
        if t.is_empty() {
            return;
        }
        Client::send_chat_message(t);
    }

    pub fn set_chat_open(&mut self, open: bool) {
        if self.chat_open == open || self.status != Status::Connected {
            self.chat_open = open;
            return;
        }
        self.chat_open = open;
        if open {
            Client::join_chat();
        } else {
            Client::leave_chat();
        }
    }

    pub fn set_sound_muted(&mut self, muted: bool) {
        self.sound_muted = muted;
        if let Some(p) = &self.player {
            p.set_muted(muted);
        }
    }

    pub fn set_mic_muted(&mut self, muted: bool) {
        self.mic_muted = muted;
        self.vox.set_muted(muted);
        match self.transmit_mode {
            TransmitMode::Ptt if muted => self.stop_talk(),
            TransmitMode::Vox => {
                let was_open = VOX
                    .lock()
                    .ok()
                    .and_then(|mut runtime| {
                        runtime.as_mut().map(|(gate, open)| {
                            gate.set_muted(muted);
                            let was_open = *open;
                            if muted && was_open {
                                gate.reset();
                                *open = false;
                            }
                            was_open
                        })
                    })
                    .unwrap_or(false);
                if was_open && muted {
                    if let Ok(mut tx) = VOX_TX.lock() {
                        if tx.transmitting {
                            Client::stop_transmit();
                            tx.transmitting = false;
                        }
                    }
                    self.vox_open = false;
                    self.transmitting = false;
                }
            }
            TransmitMode::Ptt => {}
        }
    }

    pub fn set_transmit_mode(&mut self, mode: TransmitMode) {
        if self.transmit_mode == mode {
            return;
        }
        match self.transmit_mode {
            TransmitMode::Ptt => self.stop_talk(),
            TransmitMode::Vox => self.stop_vox_capture(),
        }
        self.transmit_mode = mode;
        self.config.transmit_mode = match mode {
            TransmitMode::Ptt => "ptt".into(),
            TransmitMode::Vox => "vox".into(),
        };
        self.persist();
        self.apply_vox_state();
    }

    pub fn set_vox_sensitivity(&mut self, db: f32) {
        self.config.vox_sensitivity = db;
        self.vox.config.open_threshold_dbfs = db;
        self.vox.config.close_threshold_dbfs = db - 10.0;
        if let Ok(mut runtime) = VOX.lock() {
            if let Some((gate, _)) = runtime.as_mut() {
                gate.config.open_threshold_dbfs = db;
                gate.config.close_threshold_dbfs = db - 10.0;
            }
        }
        self.persist();
    }

    fn apply_vox_state(&mut self) {
        if self.status == Status::Connected && self.transmit_mode == TransmitMode::Vox {
            self.start_vox_capture();
        } else {
            self.stop_vox_capture();
        }
    }

    fn start_vox_capture(&mut self) {
        if self.capture.is_some() {
            return;
        }
        self.vox.reset();
        self.vox_open = false;
        {
            let mut g = VOX.lock().unwrap();
            let mut gate = VoxGate::new(self.vox.config.clone());
            gate.set_muted(self.mic_muted);
            *g = Some((gate, false));
        }
        if let Ok(mut tx) = VOX_TX.lock() {
            tx.enabled = true;
            tx.transmitting = false;
        }
        match Capture::start(nonempty(&self.config.input_device), {
            // Direct VOX path: start/stop transmit from the audio thread.
            move |pcm, rate| {
                // Safety: Client send/start/stop are mutexed.
                // We cannot touch Session from this thread; send PCM through Client
                // only when a thread-local gate says so — use a static gate.
                vox_audio_thread(pcm, rate);
            }
        }) {
            Ok(c) => self.capture = Some(c),
            Err(e) => {
                if let Ok(mut runtime) = VOX.lock() {
                    *runtime = None;
                }
                vox_tx_shutdown();
                self.last_error = Some(format!("microphone: {e}"));
            }
        }
    }

    fn stop_vox_capture(&mut self) {
        let had_gate = VOX
            .lock()
            .ok()
            .and_then(|mut runtime| runtime.take())
            .is_some();
        // Disarms the transmit side first, so an action the audio thread has
        // already decided on cannot re-open transmit behind us.
        vox_tx_shutdown();
        if had_gate || self.vox_open {
            self.capture = None;
            self.transmitting = false;
        }
        self.vox_open = false;
    }

    fn ptt_down(&mut self) {
        if self.status != Status::Connected
            || self.transmit_mode != TransmitMode::Ptt
            || self.transmitting
            || self.mic_muted
        {
            return;
        }
        Client::start_transmit();
        match Capture::start(nonempty(&self.config.input_device), |pcm, rate| {
            Client::send_pcm(pcm, rate, false);
        }) {
            Ok(c) => {
                self.capture = Some(c);
                self.transmitting = true;
            }
            Err(e) => {
                Client::stop_transmit();
                self.last_error = Some(format!("microphone: {e}"));
            }
        }
    }

    fn ptt_up(&mut self) {
        if self.transmit_mode != TransmitMode::Ptt || !self.transmitting {
            return;
        }
        self.stop_talk();
    }

    fn stop_talk(&mut self) {
        if self.transmit_mode == TransmitMode::Vox {
            return;
        }
        self.capture = None;
        if self.transmitting {
            Client::stop_transmit();
            self.transmitting = false;
        }
    }

    #[allow(dead_code)]
    pub fn page(&self, user: &User) {
        Client::send_page(user.id);
    }

    #[allow(dead_code)]
    pub fn toggle_user_mute(&mut self, id: u16) {
        if let Some(p) = &self.player {
            // flip based on a set in roster talking? keep it simple — always mute toggle via player
            p.set_user_muted(id, true);
        }
        let _ = id;
    }

    pub fn begin_ptt_capture(&self) {
        if let Some(p) = &self.ptt {
            p.begin_capture();
        }
    }
}

fn nonempty(s: &str) -> Option<&str> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

use std::sync::Mutex;
use vent_core::VoxGate as StaticGate;

static VOX: Mutex<Option<(StaticGate, bool)>> = Mutex::new(None);

/// Transmit side of VOX, kept in its own lock so the audio thread never holds
/// [`VOX`] — which the GTK loop reads every frame — across a blocking send.
struct VoxTx {
    /// False once capture is torn down, so a decision the audio thread already
    /// took cannot re-open transmit afterwards.
    enabled: bool,
    transmitting: bool,
}

static VOX_TX: Mutex<VoxTx> = Mutex::new(VoxTx {
    enabled: false,
    transmitting: false,
});

/// Stop transmitting if VOX had it open. Caller must not hold [`VOX`].
fn vox_tx_shutdown() {
    if let Ok(mut tx) = VOX_TX.lock() {
        tx.enabled = false;
        if tx.transmitting {
            Client::stop_transmit();
            tx.transmitting = false;
        }
    }
}

fn vox_audio_thread(pcm: &[u8], rate: u32) {
    // Run the gate under its lock, then release it before touching the network:
    // Client::send_pcm reaches into libventrilo3 and can block, and the GTK
    // loop takes VOX every frame for the level meter.
    let action = {
        let Ok(mut g) = VOX.lock() else {
            return;
        };
        let Some((gate, open)) = g.as_mut() else {
            return;
        };
        let action = gate.process(pcm, rate);
        match action {
            VoxAction::Open(_) => *open = true,
            VoxAction::Close => *open = false,
            _ => {}
        }
        action
    };

    let Ok(mut tx) = VOX_TX.lock() else {
        return;
    };
    if !tx.enabled {
        return;
    }
    match action {
        VoxAction::Idle => {}
        VoxAction::Open(chunks) => {
            Client::start_transmit();
            tx.transmitting = true;
            for c in chunks {
                Client::send_pcm(&c.pcm, c.rate, false);
            }
        }
        VoxAction::Transmit(c) => {
            if tx.transmitting {
                Client::send_pcm(&c.pcm, c.rate, false);
            }
        }
        VoxAction::Close => {
            if tx.transmitting {
                Client::stop_transmit();
                tx.transmitting = false;
            }
        }
    }
}
