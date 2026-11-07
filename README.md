# Audio Switcher

**Switch your speakers, headphones, and microphones right from your Deck.**

Audio Switcher is a Linux plugin for Stream Deck-style devices. It's for those times when you want to move from speakers to a headset, or change microphones, without digging through sound settings.

I built it for the Deck plugin host in [OpenLinkHub Nexus](https://github.com/avonderau/OpenLinkHub-Nexus). It follows the standard Stream Deck SDK, so it should also run in OpenDeck and the Elgato Stream Deck app on Linux, though those hosts haven't been verified here.

**Independent project:** Audio Switcher is not part of [OpenActionAPI](https://github.com/OpenActionAPI) and is not affiliated with, maintained by, or endorsed by the OpenActionAPI project. The similar naming does not indicate an official connection.

## What it does

You get two actions to put on your Deck:

- **Output device** switches your default playback device (speakers, headphones, HDMI, and so on).
- **Input device** switches your default microphone or other recording device.

You can dedicate a key to a single device, or put several devices on one key and cycle through them. For example, I can have separate keys for speakers and a headset, or one key that toggles between the two. The active device lights up on the Deck, and the label shows which one is selected.

The plugin also watches for changes made outside the Deck. If you switch your default audio device in Linux settings, your keys update to match.

### What happens when a device disconnects?

Your device choices are saved, even when a headset or microphone is unplugged. When cycling through several devices, the plugin skips ones it can't find.

There's also a **Find the same device after it is plugged back in** option, enabled by default on each key. Some USB and Bluetooth devices come back with a slightly different name after being moved to another port, reconnected, or switched between profiles. This option looks for a match using the device's name or saved description. An exact name match takes priority, and a connected device won't be matched to more than one saved choice.

If you need a key to work only with one exact device, turn that option off.

### Can a key show what kind of device it is?

Yes. In the key's settings, each device has an icon picker: speakers, headphones, headset, monitor/TV, microphone, or webcam. The key shows the icon for the device it's switching to. On a key that cycles through several devices, the icon changes to match whichever device is in use. The icon is grey when the device isn't active and green when it is. Leave it on **Default icon** to keep the plugin's standard speaker or microphone image.

### What about apps that are already playing?

By default, a key only changes the system default device. Apps set to "Default" follow it, and apps you've pointed at a specific device (like TeamSpeak or Zoom) stay where they are.

Some apps keep playing on the old device anyway, such as an RDP client like Remmina or FreeRDP. For those, turn on **Also move apps that are playing on the old device** for that key. A press then also moves every app that was on the previous default to the new one. Apps on other devices are never touched, and anything on the **Never move these apps** list stays put. That list matches part of an app's name, and it starts as `TeamSpeak, Zoom, Discord`.

## Requirements

- Linux on **x86_64** or **aarch64 (ARM64)**.
- **PulseAudio**, or **PipeWire** with `pipewire-pulse`.
- The `pactl` command.
- A compatible Deck plugin host, such as OpenLinkHub Nexus.

On Debian and Ubuntu, `pactl` is provided by `pulseaudio-utils`; on Arch, it's in `libpulse`.

You can check that your desktop audio server is responding with:

```sh
pactl info
pactl list short sinks
pactl list short sources
```

The plugin must run as **your desktop user**, since it needs access to your audio session.

## Install in OpenLinkHub Nexus

1. Download `com.avonderau.audioswitcher.streamDeckPlugin` from the [latest release](https://github.com/avonderau/OpenAction-AudioSwitcher/releases/latest).
2. Open the **Deck editor** in OpenLinkHub Nexus, select the **Plugins** tab, choose the downloaded file, and click **Install plugin**. Confirm the installation on your desktop.
3. Add an Audio Switcher action to a key using **Plugin action (experimental)**, then choose **Save & Apply**.
4. Open **Plugin settings** and select the device or devices that key should control. Give them shorter labels if their full names are too long for the display.

You can also install the downloaded package from a terminal as your desktop user:

```sh
/opt/OpenLinkHub/OpenLinkHub --install-deck-plugin /path/to/com.avonderau.audioswitcher.streamDeckPlugin
```

If you're using another Deck host, follow its own plugin installation instructions.

## How it works

Under the hood, Audio Switcher uses `pactl` to set the system's default audio sink (output) or source (input). It works with PulseAudio and with PipeWire's PulseAudio-compatible service. It also listens for audio changes through `pactl subscribe`, which keeps the buttons in sync.

Changing the default doesn't necessarily move applications you've manually pinned to another device in tools like `pavucontrol`. Those applications may stay where you put them.

If something isn't ready, a key may display:

| Message | What it means |
| --- | --- |
| **Pick device** | Select at least one device in the key's settings. |
| **No audio server** | The plugin can't reach PulseAudio or PipeWire. |
| **Install pactl** | The `pactl` command is missing. |

A failed switch also produces an alert on the Deck.

## Build it yourself

Audio Switcher is written in **Rust 1.75+**. Run the tests with:

```sh
cargo test
```

To package the plugin for x86_64 and ARM64:

```sh
rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
sh scripts/pack.sh
```

The package ends up at `dist/com.avonderau.audioswitcher.streamDeckPlugin`. The ARM64 build uses `rust-lld`, which comes with Rust.

To regenerate the key icons, install Python's Pillow library and run:

```sh
python3 scripts/make-icons.py
```

### Project layout

| Path | What you'll find |
| --- | --- |
| `src/pactl.rs` | Audio device discovery, switching, and change notifications. |
| `src/plugin.rs` | Key behavior, labels, and Deck state. |
| `src/deck.rs` | The WebSocket connection to the Deck host. |
| `com.avonderau.audioswitcher.sdPlugin/` | The plugin manifest, icons, and settings page. |
| `scripts/pack.sh` | The release packaging script. |

The tests include audio handling with a fake `pactl` command and plugin behavior with a fake Deck host.

## License

Audio Switcher is available under the [MIT License](LICENSE). You're welcome to use, modify, and share it; just keep the copyright and license notice with any copies.

## Troubleshooting

If the plugin can't see your devices, try `pactl info` in a terminal as the same user running the plugin. If that can't reach your audio server, the plugin can't either.

For OpenLinkHub Nexus, plugin logs are written to:

```text
~/.local/state/OpenLinkHub/deck-plugins/logs/com.avonderau.audioswitcher.log
```

For more on Nexus plugin support, see the [Deck plugin documentation](https://github.com/avonderau/OpenLinkHub-Nexus/blob/main/docs/deck-plugins.md).
