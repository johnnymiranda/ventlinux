use std::ffi::CStr;
use std::os::raw::c_char;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    pub id: u16,
    pub parent: u16,
    pub name: String,
    pub phonetic: String,
    pub comment: String,
    pub password_protected: bool,
    pub codec: u16,
    pub codec_format: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub id: u16,
    pub channel_id: u16,
    pub name: String,
    pub phonetic: String,
    pub comment: String,
    pub url: String,
    pub rank_id: u16,
    pub guest: bool,
    pub global_mute: bool,
    pub channel_mute: bool,
    pub phantom: bool,
    pub accepts_pages: bool,
    pub accepts_private_chat: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Codec {
    pub codec_id: u8,
    pub name: String,
    pub rate: u32,
    pub frame_size: u16,
}

impl Codec {
    /// Speex (3) and Opus (1, 2) — matches the HAVE_* flags we compile with.
    pub fn is_supported(&self) -> bool {
        matches!(self.codec_id, 1 | 2 | 3)
    }
}

#[derive(Clone, Debug)]
pub enum CoreEvent {
    Status { percent: u8, message: String },
    LoginCompleted,
    LoginFailed(String),
    ErrorMessage { message: String, disconnected: bool },
    ChannelUpserted(Channel),
    ChannelRemoved(u16),
    ChannelPasswordRejected(u16),
    UserUpserted(User),
    UserRemoved(u16),
    MovedToChannel(u16),
    TalkStarted { user_id: u16, rate: u32 },
    TalkEnded { user_id: u16 },
    Audio {
        user_id: u16,
        rate: u32,
        channels: u8,
        pcm: Vec<u8>,
    },
    Motd(String),
    Ping(u16),
    ChatJoined { user_id: u16 },
    ChatLeft { user_id: u16 },
    ChatMessage { user_id: u16, message: String },
    PrivateChatStarted { peer: u16 },
    PrivateChatEnded { peer: u16 },
    PrivateChatMessage {
        peer: u16,
        from_self: bool,
        message: String,
    },
    PrivateChatAway { peer: u16 },
    PrivateChatBack { peer: u16 },
    Paged { from_user: u16 },
    TtsMessage { user_id: u16, message: String },
    Disconnected,
}

pub(crate) fn cstr_array(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf
        .iter()
        .map(|&c| c as u8)
        .take_while(|&b| b != 0)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

pub(crate) unsafe fn ptr_str(p: *mut c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}
