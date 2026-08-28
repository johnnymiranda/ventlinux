use crate::event::{cstr_array, ptr_str, Channel, Codec, CoreEvent, User};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, OnceLock};
use std::thread;
use vent_sys as v3;

static RUNNING: AtomicBool = AtomicBool::new(false);
static CMD: OnceLock<Mutex<()>> = OnceLock::new();

fn cmd() -> &'static Mutex<()> {
    CMD.get_or_init(|| Mutex::new(()))
}

/// Audio frames from the consumer thread. Keep this off the UI loop.
pub type AudioSink = Box<dyn Fn(u16, u32, u8, &[u8]) + Send + Sync>;

static AUDIO_SINK: OnceLock<Mutex<Option<AudioSink>>> = OnceLock::new();

fn audio_sink_slot() -> &'static Mutex<Option<AudioSink>> {
    AUDIO_SINK.get_or_init(|| Mutex::new(None))
}

pub struct Client;

impl Client {
    pub fn is_logged_in() -> bool {
        unsafe { v3::v3_is_loggedin() != 0 }
    }

    pub fn own_user_id() -> u16 {
        unsafe { v3::v3_get_user_id() }
    }

    pub fn set_audio_sink<F>(f: F)
    where
        F: Fn(u16, u32, u8, &[u8]) + Send + Sync + 'static,
    {
        *audio_sink_slot().lock().unwrap() = Some(Box::new(f));
    }

    pub fn clear_audio_sink() {
        *audio_sink_slot().lock().unwrap() = None;
    }

    /// Connect and log in. The returned channel finishes on disconnect or login failure.
    pub fn connect(
        host: &str,
        port: u16,
        username: &str,
        password: &str,
        phonetic: &str,
    ) -> Receiver<CoreEvent> {
        if RUNNING.swap(true, Ordering::SeqCst) {
            let (tx, rx) = mpsc::channel();
            drop(tx);
            return rx;
        }

        unsafe { v3::v3_clear_events() };

        if let Ok(dbg) = std::env::var("V3_DEBUG") {
            let mut mask = v3::V3_DEBUG_INFO | v3::V3_DEBUG_SOCKET | v3::V3_DEBUG_ERROR;
            if dbg == "2" {
                mask |= v3::V3_DEBUG_INTERNAL | v3::V3_DEBUG_PACKET | v3::V3_DEBUG_PACKET_PARSE;
            }
            if dbg == "3" {
                mask |= v3::V3_DEBUG_INTERNAL | v3::V3_DEBUG_EVENT | v3::V3_DEBUG_MUTEX;
            }
            unsafe { v3::v3_debuglevel(mask) };
        }

        let (tx, rx) = mpsc::channel();
        let server = format!("{host}:{port}");
        let username = username.to_string();
        let password = password.to_string();
        let phonetic = phonetic.to_string();

        thread::Builder::new()
            .name("v3-feeder".into())
            .stack_size(1 << 21)
            .spawn(move || feeder(tx, server, username, password, phonetic))
            .expect("spawn feeder");
        rx
    }

    pub fn disconnect() {
        let _g = cmd().lock().unwrap();
        unsafe { v3::v3_logout() };
    }

    pub fn join_channel(id: u16, password: &str) {
        let p = CString::new(password).unwrap_or_default();
        let _g = cmd().lock().unwrap();
        unsafe { v3::v3_change_channel(id, p.as_ptr() as *mut c_char) };
    }

    pub fn channel_requires_password(id: u16) -> bool {
        unsafe { v3::v3_channel_requires_password(id) != 0 }
    }

    pub fn codec_for_channel(id: u16) -> Option<Codec> {
        unsafe {
            let c = v3::v3_get_channel_codec(id);
            if c.is_null() {
                return None;
            }
            Some(Codec {
                codec_id: (*c).codec,
                name: cstr_array(&(*c).name),
                rate: (*c).rate,
                frame_size: (*c).pcmframesize,
            })
        }
    }

    pub fn start_transmit() {
        let _g = cmd().lock().unwrap();
        unsafe { v3::v3_start_audio(v3::V3_AUDIO_SENDTYPE_U2CCUR as u16) };
    }

    pub fn send_pcm(pcm: &[u8], rate: u32, stereo: bool) {
        if pcm.is_empty() {
            return;
        }
        let _g = cmd().lock().unwrap();
        unsafe {
            v3::v3_send_audio(
                v3::V3_AUDIO_SENDTYPE_U2CCUR as u16,
                rate,
                pcm.as_ptr() as *mut u8,
                pcm.len() as u32,
                stereo as u8,
            );
        }
    }

    pub fn stop_transmit() {
        let _g = cmd().lock().unwrap();
        unsafe { v3::v3_stop_audio() };
    }

    pub fn join_chat() {
        let _g = cmd().lock().unwrap();
        unsafe { v3::v3_join_chat() };
    }
    pub fn leave_chat() {
        let _g = cmd().lock().unwrap();
        unsafe { v3::v3_leave_chat() };
    }
    pub fn send_chat_message(message: &str) {
        if message.is_empty() {
            return;
        }
        let m = CString::new(message).unwrap_or_default();
        let _g = cmd().lock().unwrap();
        unsafe { v3::v3_send_chat_message(m.as_ptr() as *mut c_char) };
    }
    pub fn start_private_chat(user_id: u16) {
        let _g = cmd().lock().unwrap();
        unsafe { v3::v3_start_privchat(user_id) };
    }
    pub fn end_private_chat(user_id: u16) {
        let _g = cmd().lock().unwrap();
        unsafe { v3::v3_end_privchat(user_id) };
    }
    pub fn send_private_chat_message(user_id: u16, message: &str) {
        if message.is_empty() {
            return;
        }
        let m = CString::new(message).unwrap_or_default();
        let _g = cmd().lock().unwrap();
        unsafe { v3::v3_send_privchat_message(user_id, m.as_ptr() as *mut c_char) };
    }
    pub fn send_page(user_id: u16) {
        let _g = cmd().lock().unwrap();
        unsafe { v3::v3_send_user_page(user_id) };
    }
    pub fn phantom_add(channel_id: u16) {
        let _g = cmd().lock().unwrap();
        unsafe { v3::v3_phantom_add(channel_id) };
    }
    pub fn phantom_remove(channel_id: u16) {
        let _g = cmd().lock().unwrap();
        unsafe { v3::v3_phantom_remove(channel_id) };
    }
    pub fn set_text(comment: &str, url: &str, silent: bool) {
        let c = CString::new(comment).unwrap_or_default();
        let u = CString::new(url).unwrap_or_default();
        let i = CString::new("").unwrap();
        let _g = cmd().lock().unwrap();
        unsafe {
            v3::v3_set_text(
                c.as_ptr() as *mut c_char,
                u.as_ptr() as *mut c_char,
                i.as_ptr() as *mut c_char,
                silent as u8,
            )
        };
    }
    pub fn set_user_volume(user_id: u16, level: i32) {
        let _g = cmd().lock().unwrap();
        unsafe { v3::v3_set_volume_user(user_id, level) };
    }
    pub fn user_volume(user_id: u16) -> u8 {
        unsafe { v3::v3_get_volume_user(user_id) }
    }
}

fn last_error() -> String {
    unsafe {
        let p = v3::v3_last_error();
        if p.is_null() {
            "unknown error".into()
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

fn feeder(
    tx: mpsc::Sender<CoreEvent>,
    server: String,
    username: String,
    password: String,
    phonetic: String,
) {
    let s = CString::new(server).unwrap();
    let u = CString::new(username).unwrap();
    let p = CString::new(password).unwrap();
    let ph = CString::new(phonetic).unwrap();

    unsafe {
        // Force the event-queue mutex to exist before login (see VentMac V3Client).
        let stale = v3::v3_get_event(v3::V3_NONBLOCK as i32);
        if !stale.is_null() {
            v3::v3_free_event(stale);
        }
        if v3::v3_login(
            s.as_ptr() as *mut c_char,
            u.as_ptr() as *mut c_char,
            p.as_ptr() as *mut c_char,
            ph.as_ptr() as *mut c_char,
        ) == 0
        {
            let _ = tx.send(CoreEvent::LoginFailed(last_error()));
            RUNNING.store(false, Ordering::SeqCst);
            return;
        }
    }

    let consumer_tx = tx.clone();
    thread::Builder::new()
        .name("v3-consumer".into())
        .stack_size(1 << 21)
        .spawn(move || consumer(consumer_tx))
        .expect("spawn consumer");

    loop {
        let msg = unsafe { v3::_v3_recv(v3::V3_BLOCK as i32) };
        if msg.is_null() {
            break;
        }
        unsafe { v3::_v3_process_message(msg) };
    }
}

fn consumer(tx: mpsc::Sender<CoreEvent>) {
    loop {
        let ev = unsafe { v3::v3_get_event(v3::V3_BLOCK as i32) };
        if ev.is_null() {
            break;
        }
        let translated = unsafe { translate(ev) };
        unsafe { v3::v3_free_event(ev) };
        if let Some(event) = translated {
            let done = matches!(event, CoreEvent::Disconnected);
            if tx.send(event).is_err() {
                break;
            }
            if done {
                break;
            }
        }
    }
    RUNNING.store(false, Ordering::SeqCst);
}

unsafe fn read_user(id: u16) -> Option<User> {
    let p = v3::v3_get_user(id);
    if p.is_null() {
        return None;
    }
    let u = &*p;
    let user = User {
        id: u.id,
        channel_id: u.channel,
        name: ptr_str(u.name),
        phonetic: ptr_str(u.phonetic),
        comment: ptr_str(u.comment),
        url: ptr_str(u.url),
        rank_id: u.rank_id,
        guest: u.guest != 0,
        global_mute: u.global_mute != 0,
        channel_mute: u.channel_mute != 0,
        phantom: u.real_user_id != 0,
        accepts_pages: u.accept_pages != 0,
        accepts_private_chat: u.accept_u2u != 0,
    };
    v3::v3_free_user(p);
    Some(user)
}

unsafe fn read_channel(id: u16) -> Option<Channel> {
    let p = v3::v3_get_channel(id);
    if p.is_null() {
        return None;
    }
    let c = &*p;
    let ch = Channel {
        id: c.id,
        parent: c.parent,
        name: ptr_str(c.name),
        phonetic: ptr_str(c.phonetic),
        comment: ptr_str(c.comment),
        password_protected: c.password_protected != 0,
        codec: c.channel_codec,
        codec_format: c.channel_format,
    };
    v3::v3_free_channel(p);
    Some(ch)
}

unsafe fn event_chat_message(ev: &v3::_v3_event) -> String {
    if ev.data.is_null() {
        return String::new();
    }
    ptr_str((*ev.data).chatmessage.as_ptr() as *mut c_char)
}

unsafe fn privchat_peer(ev: &v3::_v3_event) -> u16 {
    let me = v3::v3_get_user_id();
    if ev.user.privchat_user1 == me {
        ev.user.privchat_user2
    } else {
        ev.user.privchat_user1
    }
}

unsafe fn translate(ev: *mut v3::_v3_event) -> Option<CoreEvent> {
    let e = &*ev;
    let ty = e.type_ as u32;
    match ty {
        t if t == v3::_v3_events_V3_EVENT_STATUS => Some(CoreEvent::Status {
            percent: e.status.percent,
            message: cstr_array(&e.status.message),
        }),
        t if t == v3::_v3_events_V3_EVENT_LOGIN_COMPLETE => Some(CoreEvent::LoginCompleted),
        t if t == v3::_v3_events_V3_EVENT_LOGIN_FAIL => {
            Some(CoreEvent::LoginFailed(cstr_array(&e.error.message)))
        }
        t if t == v3::_v3_events_V3_EVENT_ERROR_MSG => Some(CoreEvent::ErrorMessage {
            message: cstr_array(&e.error.message),
            disconnected: e.error.disconnected != 0,
        }),
        t if t == v3::_v3_events_V3_EVENT_CHAN_ADD
            || t == v3::_v3_events_V3_EVENT_CHAN_MODIFY
            || t == v3::_v3_events_V3_EVENT_CHAN_MODIFIED =>
        {
            read_channel(e.channel.id).map(CoreEvent::ChannelUpserted)
        }
        t if t == v3::_v3_events_V3_EVENT_CHAN_REMOVE
            || t == v3::_v3_events_V3_EVENT_CHAN_REMOVED =>
        {
            Some(CoreEvent::ChannelRemoved(e.channel.id))
        }
        t if t == v3::_v3_events_V3_EVENT_CHAN_BADPASS => {
            Some(CoreEvent::ChannelPasswordRejected(e.channel.id))
        }
        t if t == v3::_v3_events_V3_EVENT_USER_LOGIN
            || t == v3::_v3_events_V3_EVENT_USER_MODIFY
            || t == v3::_v3_events_V3_EVENT_USER_CHAN_MOVE =>
        {
            read_user(e.user.id).map(CoreEvent::UserUpserted)
        }
        t if t == v3::_v3_events_V3_EVENT_USER_LOGOUT => Some(CoreEvent::UserRemoved(e.user.id)),
        t if t == v3::_v3_events_V3_EVENT_CHANGE_CHANNEL => {
            Some(CoreEvent::MovedToChannel(e.channel.id))
        }
        t if t == v3::_v3_events_V3_EVENT_USER_TALK_START => Some(CoreEvent::TalkStarted {
            user_id: e.user.id,
            rate: e.pcm.rate,
        }),
        t if t == v3::_v3_events_V3_EVENT_USER_TALK_END
            || t == v3::_v3_events_V3_EVENT_USER_TALK_MUTE =>
        {
            Some(CoreEvent::TalkEnded {
                user_id: e.user.id,
            })
        }
        t if t == v3::_v3_events_V3_EVENT_PLAY_AUDIO => {
            if e.data.is_null() {
                return None;
            }
            let max = std::mem::size_of::<v3::v3_event_data>();
            let len = (e.pcm.length as usize).min(max);
            let pcm = std::slice::from_raw_parts(e.data as *const u8, len);
            if let Ok(sink) = audio_sink_slot().lock() {
                if let Some(cb) = sink.as_ref() {
                    cb(e.user.id, e.pcm.rate, e.pcm.channels, pcm);
                    return None;
                }
            }
            Some(CoreEvent::Audio {
                user_id: e.user.id,
                rate: e.pcm.rate,
                channels: e.pcm.channels,
                pcm: pcm.to_vec(),
            })
        }
        t if t == v3::_v3_events_V3_EVENT_DISPLAY_MOTD => {
            if e.data.is_null() {
                return None;
            }
            Some(CoreEvent::Motd(ptr_str(
                (*e.data).motd.as_ptr() as *mut c_char
            )))
        }
        t if t == v3::_v3_events_V3_EVENT_PING => Some(CoreEvent::Ping(e.ping)),
        t if t == v3::_v3_events_V3_EVENT_CHAT_JOIN => {
            Some(CoreEvent::ChatJoined { user_id: e.user.id })
        }
        t if t == v3::_v3_events_V3_EVENT_CHAT_LEAVE => {
            Some(CoreEvent::ChatLeft { user_id: e.user.id })
        }
        t if t == v3::_v3_events_V3_EVENT_CHAT_MESSAGE => Some(CoreEvent::ChatMessage {
            user_id: e.user.id,
            message: event_chat_message(e),
        }),
        t if t == v3::_v3_events_V3_EVENT_PRIVATE_CHAT_START => Some(CoreEvent::PrivateChatStarted {
            peer: privchat_peer(e),
        }),
        t if t == v3::_v3_events_V3_EVENT_PRIVATE_CHAT_END => Some(CoreEvent::PrivateChatEnded {
            peer: privchat_peer(e),
        }),
        t if t == v3::_v3_events_V3_EVENT_PRIVATE_CHAT_MESSAGE => {
            let me = v3::v3_get_user_id();
            Some(CoreEvent::PrivateChatMessage {
                peer: privchat_peer(e),
                from_self: e.user.privchat_user2 == me,
                message: event_chat_message(e),
            })
        }
        t if t == v3::_v3_events_V3_EVENT_PRIVATE_CHAT_AWAY => Some(CoreEvent::PrivateChatAway {
            peer: privchat_peer(e),
        }),
        t if t == v3::_v3_events_V3_EVENT_PRIVATE_CHAT_BACK => Some(CoreEvent::PrivateChatBack {
            peer: privchat_peer(e),
        }),
        t if t == v3::_v3_events_V3_EVENT_USER_PAGE => Some(CoreEvent::Paged {
            from_user: e.user.id,
        }),
        t if t == v3::_v3_events_V3_EVENT_TEXT_TO_SPEECH_MESSAGE => Some(CoreEvent::TtsMessage {
            user_id: e.user.id,
            message: event_chat_message(e),
        }),
        t if t == v3::_v3_events_V3_EVENT_USER_GLOBAL_MUTE_CHANGED
            || t == v3::_v3_events_V3_EVENT_USER_CHANNEL_MUTE_CHANGED =>
        {
            read_user(e.user.id).map(CoreEvent::UserUpserted)
        }
        t if t == v3::_v3_events_V3_EVENT_DISCONNECT => Some(CoreEvent::Disconnected),
        _ => None,
    }
}
