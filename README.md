# Audio Switcher for OpenDeck

Deck keys that switch your sound output (speakers, headphones, HDMI) and
your microphone. The plugin is written for the Deck plugin host in
[OpenLinkHub Nexus](https://github.com/avonderau/OpenLinkHub-Nexus)
([docs](https://github.com/avonderau/OpenLinkHub-Nexus/blob/main/docs/deck-plugins.md)).
It is a standard Stream Deck SDK plugin, so it should also run in OpenDeck
and the Elgato Stream Deck app on Linux.

| Key | Press | Shows |
| --- | --- | --- |
| Output device | Switch to the key's speakers/headphones, or to the next of several | The device; lit while it is in use |
| Input device | Switch to the key's microphone, or to the next of several | The device; lit while it is in use |

Give a key **one device** and a press makes it the default; the key lights
while it is. Give it **several** and each press moves to the next one that
is plugged in; the key shows the device in use and lights while it is one
of the key's devices.

**Find the same device after it is plugged back in** (on by default, per
key) covers devices that come back under a different name, such as a USB
headset in another port, PipeWire's `.2` suffix on a re-plugged device, or
a Bluetooth headset that switched profile. When a saved device is missing,
the key uses a connected one whose name differs only in its number parts,
or whose description matches the one saved with the key. A device with its
exact saved name always wins, and each connected device stands in for at
most one saved device. Turn it off for keys that must only ever use one
specific device.

Keys show "Pick device" until you choose one, "No audio server" when
PulseAudio or PipeWire is not reachable, and "Install pactl" when the
`pactl` command is missing. A press on a device that is not plugged in
shows an alert.

## How it works

The plugin runs `pactl`, which talks to PulseAudio, or to PipeWire through
pipewire-pulse (the default on current Fedora, Ubuntu, Debian and Arch
desktops). It sets the default sink or source, which is what the desktop
sound settings change, and keeps `pactl subscribe` running so the keys
update when you switch devices anywhere else.

## Requirements

- Linux on x86_64 or aarch64, with PulseAudio or PipeWire (pipewire-pulse).
- `pactl`: the `pulseaudio-utils` package on Debian, Ubuntu and Fedora,
  `libpulse` on Arch. The plugin itself is a single static binary.

## Install

1. Download `com.avonderau.audioswitcher.streamDeckPlugin` from the
   [latest release](https://github.com/avonderau/openaction-audioswitcher/releases/latest),
   or build it yourself (see below).
2. In OpenLinkHub, open the Deck editor, go to the **Plugins** tab, choose
   the file and click **Install plugin**, then confirm on your desktop. Or,
   as your desktop user:

   ```sh
   /opt/OpenLinkHub/OpenLinkHub --install-deck-plugin dist/com.avonderau.audioswitcher.streamDeckPlugin
   ```

3. Put an Audio Switcher action on a key (**Plugin action (experimental)**),
   **Save & Apply**, then open **Plugin settings**.
4. Tick the device (or devices) for the key. Ticked devices stay in the
   list while unplugged, so a headset keeps its place. Fill in **Key label**
   for a short name; device names are often long.

The plugin must run as your desktop user, since `pactl` talks to the audio
server of the user that runs it.

## Development

The plugin is written in Rust (stable, 1.75 or later).

```sh
cargo test             # unit tests, the worker against a fake pactl, and
                       # the binary driven through a fake Deck host
sh scripts/pack.sh     # writes dist/com.avonderau.audioswitcher.streamDeckPlugin
python3 scripts/make-icons.py   # redraws the key icons (needs Pillow)
```

`scripts/pack.sh` builds static binaries for both architectures and needs
`rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl`.
The aarch64 build links with `rust-lld`, which ships with Rust.

| Path | What it is |
| --- | --- |
| `src/pactl.rs` | `pactl` worker: device lists, defaults, change events |
| `src/plugin.rs` | Key actions: what each key does and shows |
| `src/deck.rs` | WebSocket connection to the Deck host |
| `com.avonderau.audioswitcher.sdPlugin/` | Manifest, icons and settings page |

## Notes

- Apps that were pinned to a device (for example in pavucontrol) stay on
  it; everything else follows the new default.
- Plugin output goes to
  `~/.local/state/OpenLinkHub/deck-plugins/logs/com.avonderau.audioswitcher.log`.
