# VentLinux

A native Linux client for legacy **Ventrilo 3** voice servers, with a real
system-wide push-to-talk key — the thing that never worked under Wine.

VentLinux is an independent, open-source client built on
[Mangler](https://github.com/econnell/mangler)'s `libventrilo3` protocol
library. It is the Linux counterpart of
[VentMac](https://github.com/johnnymiranda/ventmac) and
[VentiPhone](https://github.com/johnnymiranda/ventiphone): same protocol core,
PipeWire/Pulse audio, and global PTT via evdev so it still works when a
fullscreen / Proton game has focus.

## Why this exists

The same group has been on Ventrilo 3 for years. VentMac covers macOS;
VentiPhone covers iOS. Linux was the remaining seat at the table — and Wine
still can't bind a reliable global PTT key while a game is focused. So this
is that client.

> **Not affiliated with Ventrilo.** VentLinux is an independent interoperability
> project. It is not affiliated with, sponsored by, or endorsed by LightSpeed
> Gaming LLC (the current owner of the Ventrilo trademark) or the former
> Flagship Industries, Inc. "Ventrilo" is a trademark of its respective owner
> and is used here only descriptively, to state compatibility. Use VentLinux
> only to connect to servers you are authorized to use.

## Features

- Native GTK4 / libadwaita app — no Wine, no Windows binaries
- Connects to Ventrilo 3.x servers (Speex and Opus)
- Channel tree with users and live talk indicators
- **Global push-to-talk** from a keyboard key or mouse side-button, including
  over fullscreen games ([details](#global-ptt))
- **Voice activation (VOX)** as an alternative to PTT, with a sensitivity slider
- **Auto-reconnect** — dropped connections retry with backoff and rejoin your channel
- Text chat, channel passwords, paging, MOTD
- Saved server list (`~/.config/ventlinux/`; passwords in a `0600` file)
- Selectable microphone and output device (Pulse/PipeWire)

## Build

Needs a Rust toolchain (1.80+), clang (for bindgen), and:

```
speex speexdsp opus gtk4 libadwaita pkgconf
```

On Arch / Omarchy:

```sh
sudo pacman -S --needed rust clang speex speexdsp opus gtk4 libadwaita pkgconf
git clone https://github.com/johnnymiranda/ventlinux.git
cd ventlinux
cargo build --release
```

Binaries land in `target/release/`:

```sh
./target/release/ventlinux
./target/release/ventctl --help
```

`V3_DEBUG=1` (or `2` / `3`) enables libventrilo3 debug output.

## Connecting to a server

Open VentLinux, click **Add Server**, and fill in a name, the server **host,
port, and username** (plus a **password** if the server requires one). Select
the server and click **Connect** — or double-click it. Double-click a channel
to join it, then hold your push-to-talk key to speak (or switch to voice
activation in Preferences).

The gear in the header is **Preferences**: PTT rebind, PTT vs VOX, VOX
sensitivity, and input/output devices.

Server entries (including passwords) live in `~/.config/ventlinux/servers.json`
with mode `0600`. Other settings are in `~/.config/ventlinux/config.toml`.

## Global PTT

VentLinux listens to `/dev/input/event*` **without grabbing** devices, so games
keep their keys. Your user must be in the `input` group:

```sh
sudo usermod -aG input "$USER"
# log out and back in
```

Default bind is **Left Ctrl**. Rebind it in Preferences to another key or
mouse 4/5.

## Targets

| Crate | What |
|---|---|
| `vendor/libventrilo3` | Vendored C protocol library (Mangler + VentMac 3.1.0 handshake) |
| `vent-sys` | bindgen + cc |
| `vent-core` | Event pump, roster, VOX gate, transmit |
| `vent-audio` | cpal capture + mixed playback (Pulse/PipeWire) |
| `vent-ptt` | evdev global PTT |
| `ventctl` | Headless CLI: connect, dump channel tree, listen, Enter-to-talk, `devices` |
| `ventlinux` | GTK4 app: server list, channel tree, chat, PTT, VOX, device pickers |

```sh
./target/release/ventctl devices
./target/release/ventctl -s host:3784 -u name --stay --talk
```

## macOS / iOS siblings

- [VentMac](https://github.com/johnnymiranda/ventmac) — SwiftUI, global PTT
- [VentiPhone](https://github.com/johnnymiranda/ventiphone) — iOS, VOX transmit

## Protocol notes

Handshake updates for present-day 3.1.0 servers are documented in VentMac:
[`docs/HANDSHAKE-FINDINGS.md`](https://github.com/johnnymiranda/ventmac/blob/main/docs/HANDSHAKE-FINDINGS.md).

## Contributing

Bug reports and pull requests are welcome via GitHub Issues and PRs;
contributions are under GPL-3.0. For anything protocol- or crypto-sensitive
you'd rather not post publicly, open a minimal issue and ask for a private
channel.

## Attribution

- **Mangler / libventrilo3** — © 2009–2011 Eric Connell, GPL-3.0. Vendored
  under `vendor/libventrilo3/`.
- **Ventrilo packet crypto** — reverse-engineering by Luigi Auriemma.
- 3.1.0 handshake DNS patch — [VentMac](https://github.com/johnnymiranda/ventmac).
- Linux client — this project.

## License

**GPL-3.0-or-later** — inherited from `libventrilo3`. See [`LICENSE`](LICENSE).
The vendored upstream license and copyright are retained under
`vendor/libventrilo3/`.
