//! Protocol core for a Ventrilo 3 client.
//!
//! `libventrilo3` is a process-wide singleton (one connection). [`Client`]
//! wraps that: a feeder thread runs `v3_login` / `_v3_recv`, a consumer thread
//! drains `v3_get_event` into an [`std::sync::mpsc`] channel.

mod client;
mod event;
mod roster;
mod vox;

pub use client::Client;
pub use event::{Channel, Codec, CoreEvent, User};
pub use roster::{Roster, TreeNode};
pub use vox::{VoxAction, VoxChunk, VoxConfig, VoxGate};
