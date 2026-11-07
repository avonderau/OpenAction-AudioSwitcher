//! Key logic: turns Deck events into audio requests and the audio state
//! into key titles, states and settings-page updates. It does no I/O; the
//! messages to send to the Deck host collect in [`Plugin::outbox`].
//!
//! A key has a list of devices in its settings. With one device, a press
//! makes it the default and the key lights while it is. With more, a press
//! moves to the next one in the list and the key shows the current one.

use std::collections::HashMap;
use std::sync::mpsc::Sender;

use serde_json::{json, Value};

use crate::pactl::{AudioEvent, AudioState, Command, Kind, Request, Status};

pub const PLUGIN: &str = "com.avonderau.audioswitcher";

pub fn kind_from_uuid(uuid: &str) -> Option<Kind> {
    match uuid.strip_prefix(PLUGIN)?.strip_prefix('.')? {
        "output" => Some(Kind::Output),
        "input" => Some(Kind::Input),
        _ => None,
    }
}

const MAX_LINE: usize = 10;
const MAX_LINES: usize = 2;

/// Wraps text into at most two lines of `MAX_LINE` characters, with an
/// ellipsis where it is cut.
pub fn fit(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= MAX_LINE => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    let cut = lines.len() > MAX_LINES;
    lines.truncate(MAX_LINES);
    let last = lines.len().saturating_sub(1);
    for (i, line) in lines.iter_mut().enumerate() {
        if line.chars().count() > MAX_LINE || (cut && i == last) {
            let mut short: String = line.chars().take(MAX_LINE - 1).collect();
            short.push('…');
            *line = short;
        }
    }
    lines.join("\n")
}

/// The device names a key switches between, in order.
pub fn selected(settings: &Value) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for v in settings
        .get("devices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(name) = v.as_str().map(str::trim).filter(|n| !n.is_empty()) {
            if !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// A device name with its number parts (USB interface, serial, the ".2"
/// PipeWire adds to a duplicate) blanked out, so the same device matches
/// after it was plugged back in under a slightly different name.
pub fn fingerprint(name: &str) -> String {
    let chars: Vec<char> = name.to_lowercase().chars().collect();
    let sep = |c: char| matches!(c, '-' | '.' | '_');
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() && (i == 0 || sep(chars[i - 1])) {
            let end = (i..chars.len())
                .find(|&j| !chars[j].is_ascii_digit())
                .unwrap_or(chars.len());
            if end == chars.len() || sep(chars[end]) {
                out.push('#');
                i = end;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    while let Some(rest) = out.strip_suffix(".#") {
        out.truncate(rest.len());
    }
    out
}

fn text<'a>(settings: &'a Value, map: &str, name: &str) -> &'a str {
    settings
        .get(map)
        .and_then(|m| m.get(name))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
}

/// Each of the key's devices with the connected device it stands for: the
/// device of that name, or (unless `matchSimilar` is off) a connected
/// device that looks like the same one under a new name.
pub fn resolve(kind: Kind, audio: &AudioState, settings: &Value) -> Vec<(String, Option<String>)> {
    let names = selected(settings);
    let devices = audio.devices(kind);
    let similar = settings.get("matchSimilar").and_then(Value::as_bool) != Some(false);
    let mut taken: Vec<&str> = names
        .iter()
        .filter(|n| devices.iter().any(|d| &d.name == *n))
        .map(String::as_str)
        .collect();
    let mut out = Vec::new();
    for name in &names {
        if devices.iter().any(|d| &d.name == name) {
            out.push((name.clone(), Some(name.clone())));
            continue;
        }
        let description = text(settings, "descriptions", name);
        let print = fingerprint(name);
        let best = devices
            .iter()
            .filter(|_| similar)
            .filter(|d| !taken.contains(&d.name.as_str()))
            .filter_map(|d| {
                let same_print = fingerprint(&d.name) == print;
                let same_desc = !description.is_empty() && d.description == description;
                let score = 2 * u8::from(same_print) + u8::from(same_desc);
                (score > 0).then_some((score, d))
            })
            .fold(None::<(u8, &crate::pactl::Device)>, |best, c| match best {
                Some(b) if b.0 >= c.0 => Some(b),
                _ => Some(c),
            })
            .map(|(_, d)| d);
        if let Some(d) = best {
            taken.push(&d.name);
        }
        out.push((name.clone(), best.map(|d| d.name.clone())));
    }
    out
}

/// What a key shows for one of its devices: its label from the key
/// settings, else the description of the device it stands for, else the
/// saved description, else the raw name.
fn label(
    kind: Kind,
    audio: &AudioState,
    settings: &Value,
    saved: &str,
    device: Option<&str>,
) -> String {
    let custom = text(settings, "labels", saved);
    if !custom.is_empty() {
        return custom.to_string();
    }
    if let Some(d) = device.and_then(|n| audio.devices(kind).iter().find(|d| d.name == n)) {
        return d.description.clone();
    }
    let saved_desc = text(settings, "descriptions", saved);
    if saved_desc.is_empty() {
        saved
    } else {
        saved_desc
    }
    .to_string()
}

/// The title and state a key shows.
pub fn view(kind: Kind, audio: &AudioState, settings: &Value) -> (String, u8) {
    let title = match audio.status {
        Status::Ready => None,
        Status::Starting => Some("Audio\n…"),
        Status::NoServer => Some("No audio\nserver"),
        Status::NoPactl => Some("Install\npactl"),
    };
    if let Some(title) = title {
        return (title.into(), 0);
    }
    let entries = resolve(kind, audio, settings);
    let current = audio.default_device(kind);
    match entries.as_slice() {
        [] => ("Pick\ndevice".into(), 0),
        [(saved, device)] => (
            fit(&label(kind, audio, settings, saved, device.as_deref())),
            u8::from(device.as_deref() == Some(current)),
        ),
        _ => match entries.iter().find(|e| e.1.as_deref() == Some(current)) {
            Some((saved, device)) => (
                fit(&label(kind, audio, settings, saved, device.as_deref())),
                1,
            ),
            None => (
                fit(&label(kind, audio, settings, current, Some(current))),
                0,
            ),
        },
    }
}

/// The device a press switches to, or None when none of the key's devices
/// is connected.
pub fn next_device(kind: Kind, audio: &AudioState, settings: &Value) -> Option<String> {
    let names: Vec<String> = resolve(kind, audio, settings)
        .into_iter()
        .filter_map(|e| e.1)
        .collect();
    let current = audio.default_device(kind);
    let next = match names.iter().position(|n| n == current) {
        Some(i) => (i + 1) % names.len(),
        None => 0,
    };
    names.into_iter().nth(next)
}

/// The device icons a key can show, drawn by scripts/make-icons.py.
pub const ICONS: [&str; 6] = [
    "speakers",
    "headphones",
    "headset",
    "monitor",
    "microphone",
    "webcam",
];

macro_rules! device_png {
    ($name:literal, $state:literal) => {
        include_bytes!(concat!(
            "../com.avonderau.audioswitcher.sdPlugin/imgs/devices/",
            $name,
            "-",
            $state,
            ".png"
        ))
    };
}

fn icon_png(icon: &str, on: bool) -> Option<&'static [u8]> {
    let (off, lit): (&[u8], &[u8]) = match icon {
        "speakers" => (
            device_png!("speakers", "off"),
            device_png!("speakers", "on"),
        ),
        "headphones" => (
            device_png!("headphones", "off"),
            device_png!("headphones", "on"),
        ),
        "headset" => (device_png!("headset", "off"), device_png!("headset", "on")),
        "monitor" => (device_png!("monitor", "off"), device_png!("monitor", "on")),
        "microphone" => (
            device_png!("microphone", "off"),
            device_png!("microphone", "on"),
        ),
        "webcam" => (device_png!("webcam", "off"), device_png!("webcam", "on")),
        _ => return None,
    };
    Some(if on { lit } else { off })
}

/// Standard base64, for the data URLs `setImage` takes.
pub fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ABC[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The icon picked for the device the key shows, if any: the key's one
/// device, or with several, the one in use.
pub fn icon(kind: Kind, audio: &AudioState, settings: &Value) -> Option<&'static str> {
    if audio.status != Status::Ready {
        return None;
    }
    let entries = resolve(kind, audio, settings);
    let current = audio.default_device(kind);
    let saved = match entries.as_slice() {
        [(saved, _)] => saved,
        _ => &entries.iter().find(|e| e.1.as_deref() == Some(current))?.0,
    };
    let picked = text(settings, "icons", saved);
    ICONS.into_iter().find(|i| *i == picked)
}

/// Apps that are never moved unless the key's list says otherwise:
/// voice and meeting apps usually pick their own devices.
pub const DEFAULT_KEEP: &str = "TeamSpeak, Zoom, Discord";

/// Whether a press also moves playing apps (off unless `moveApps` is
/// set), and the never-move list from `keepApps`.
pub fn move_settings(settings: &Value) -> (bool, Vec<String>) {
    let move_apps = settings.get("moveApps").and_then(Value::as_bool) == Some(true);
    let keep = settings
        .get("keepApps")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_KEEP)
        .split([',', '\n'])
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_string)
        .collect();
    (move_apps, keep)
}

struct Key {
    kind: Kind,
    settings: Value,
    shown: Option<(String, u8)>,
    /// The custom icon last set on the key, and the state it was drawn for.
    image: Option<(&'static str, u8)>,
}

pub struct Plugin {
    requests: Sender<Request>,
    keys: HashMap<String, Key>,
    /// Open settings pages: context -> action UUID.
    inspectors: HashMap<String, String>,
    audio: AudioState,
    pub outbox: Vec<Value>,
}

impl Plugin {
    pub fn new(requests: Sender<Request>) -> Plugin {
        Plugin {
            requests,
            keys: HashMap::new(),
            inspectors: HashMap::new(),
            audio: AudioState::default(),
            outbox: Vec::new(),
        }
    }

    pub fn on_deck(&mut self, msg: &Value) {
        let text = |k: &str| msg.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let (event, context, action) = (text("event"), text("context"), text("action"));
        let payload = msg.get("payload").cloned().unwrap_or(Value::Null);
        let settings = || {
            payload
                .get("settings")
                .cloned()
                .unwrap_or_else(|| json!({}))
        };
        match event.as_str() {
            "willAppear" => {
                let Some(kind) = kind_from_uuid(&action) else {
                    return;
                };
                let key = Key {
                    kind,
                    settings: settings(),
                    shown: None,
                    image: None,
                };
                self.keys.insert(context.clone(), key);
                self.render(&context);
            }
            "willDisappear" => {
                self.keys.remove(&context);
            }
            "didReceiveSettings" => {
                if let Some(key) = self.keys.get_mut(&context) {
                    key.settings = settings();
                    self.render(&context);
                    if self.inspectors.contains_key(&context) {
                        self.send_devices(&context);
                    }
                }
            }
            "keyDown" => self.on_key(&context),
            "propertyInspectorDidAppear" => {
                self.inspectors.insert(context.clone(), action);
                self.send_devices(&context);
            }
            "propertyInspectorDidDisappear" => {
                self.inspectors.remove(&context);
            }
            "sendToPlugin" => match payload.get("type").and_then(Value::as_str) {
                Some("devices") => {
                    self.inspectors.entry(context.clone()).or_insert(action);
                    self.send_devices(&context);
                }
                Some("refresh") => self.request(Command::Refresh, None),
                _ => {}
            },
            _ => {}
        }
    }

    pub fn on_audio(&mut self, event: AudioEvent) {
        match event {
            AudioEvent::State(state) => {
                self.audio = state;
                self.render_all();
                let open: Vec<String> = self.inspectors.keys().cloned().collect();
                for context in open {
                    self.send_devices(&context);
                }
            }
            AudioEvent::Failed { context, error } => {
                self.log(&format!("switch failed: {error}"));
                if let Some(context) = context {
                    self.alert(&context);
                }
            }
            AudioEvent::Log(line) => self.log(&line),
        }
    }

    pub fn log(&mut self, message: &str) {
        self.outbox
            .push(json!({"event": "logMessage", "payload": {"message": message}}));
    }

    fn request(&self, command: Command, context: Option<&str>) {
        let _ = self.requests.send(Request {
            command,
            context: context.map(str::to_string),
        });
    }

    fn alert(&mut self, context: &str) {
        self.outbox
            .push(json!({"event": "showAlert", "context": context}));
    }

    fn on_key(&mut self, context: &str) {
        let Some(key) = self.keys.get(context) else {
            return;
        };
        let kind = key.kind;
        let next = if self.audio.status == Status::Ready {
            next_device(kind, &self.audio, &key.settings)
        } else {
            None
        };
        match next {
            Some(name) => {
                let (move_apps, keep) = move_settings(&key.settings);
                self.request(
                    Command::SetDefault {
                        kind,
                        name,
                        move_apps,
                        keep,
                    },
                    Some(context),
                )
            }
            None => self.alert(context),
        }
    }

    fn send_devices(&mut self, context: &str) {
        let action = self.inspectors.get(context).cloned().unwrap_or_default();
        let Some(kind) = kind_from_uuid(&action) else {
            return;
        };
        let matches: serde_json::Map<String, Value> = self
            .keys
            .get(context)
            .map(|k| resolve(kind, &self.audio, &k.settings))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(saved, device)| {
                device
                    .filter(|d| *d != saved)
                    .map(|d| (saved, Value::String(d)))
            })
            .collect();
        self.outbox.push(json!({"event": "sendToPropertyInspector", "context": context, "action": action, "payload": {
            "type": "devices",
            "status": self.audio.status,
            "kind": kind,
            "devices": self.audio.devices(kind),
            "current": self.audio.default_device(kind),
            "matches": matches,
        }}));
    }

    fn render_all(&mut self) {
        let contexts: Vec<String> = self.keys.keys().cloned().collect();
        for context in contexts {
            self.render(&context);
        }
    }

    fn render(&mut self, context: &str) {
        let Some(key) = self.keys.get_mut(context) else {
            return;
        };
        let (title, state) = view(key.kind, &self.audio, &key.settings);
        let shown = key.shown.as_ref();
        if shown.map(|s| &s.0) != Some(&title) {
            self.outbox.push(json!({"event": "setTitle", "context": context, "payload": {"title": title, "target": 0}}));
        }
        if shown.map(|s| s.1) != Some(state) {
            self.outbox.push(
                json!({"event": "setState", "context": context, "payload": {"state": state}}),
            );
        }
        key.shown = Some((title, state));
        // Without a picked icon the key uses the manifest's state images.
        let image = icon(key.kind, &self.audio, &key.settings).map(|i| (i, state));
        if image != key.image {
            let payload = match image.and_then(|(i, s)| icon_png(i, s == 1)) {
                Some(png) => {
                    json!({"image": format!("data:image/png;base64,{}", base64(png)), "target": 0})
                }
                None => json!({"target": 0}),
            };
            self.outbox
                .push(json!({"event": "setImage", "context": context, "payload": payload}));
            key.image = image;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pactl::Device;

    const SPEAKERS: &str = "alsa_output.speakers";
    const HEADSET: &str = "alsa_output.headset";
    const HDMI: &str = "alsa_output.hdmi";

    fn audio() -> AudioState {
        let dev = |name: &str, description: &str| Device {
            name: name.into(),
            description: description.into(),
        };
        AudioState {
            status: Status::Ready,
            outputs: vec![
                dev(SPEAKERS, "Speakers"),
                dev(HEADSET, "G535 Wireless Gaming Headset"),
                dev(HDMI, "HDMI"),
            ],
            inputs: vec![dev("alsa_input.mic", "Desk Mic")],
            default_output: SPEAKERS.into(),
            default_input: "alsa_input.mic".into(),
        }
    }

    #[test]
    fn fits_titles_on_a_key() {
        assert_eq!(fit("Speakers"), "Speakers");
        assert_eq!(fit("Built-in Audio Analog Stereo"), "Built-in\nAudio…");
        assert_eq!(fit("G535 Wireless Gaming Headset"), "G535\nWireless…");
        assert_eq!(fit("Supercalifragilistic"), "Supercali…");
        assert_eq!(fit("  HDMI   1  "), "HDMI 1");
        assert_eq!(fit(""), "");
    }

    #[test]
    fn not_ready_states_show_on_every_key() {
        for (status, title) in [
            (Status::Starting, "Audio\n…"),
            (Status::NoServer, "No audio\nserver"),
            (Status::NoPactl, "Install\npactl"),
        ] {
            let a = AudioState::with_status(status);
            for kind in [Kind::Output, Kind::Input] {
                assert_eq!(
                    view(kind, &a, &json!({"devices": [SPEAKERS]})),
                    (title.into(), 0)
                );
            }
        }
    }

    #[test]
    fn single_device_keys() {
        let a = audio();
        let s = json!({"devices": [SPEAKERS]});
        assert_eq!(view(Kind::Output, &a, &s), ("Speakers".into(), 1));
        let s = json!({"devices": [HEADSET], "labels": {HEADSET: "Headset"}});
        assert_eq!(view(Kind::Output, &a, &s), ("Headset".into(), 0));
        assert_eq!(next_device(Kind::Output, &a, &s).as_deref(), Some(HEADSET));
        let s = json!({"devices": ["alsa_output.unplugged"]});
        assert_eq!(
            view(Kind::Output, &a, &s).0,
            "alsa_outp…",
            "a missing device still shows its name"
        );
        assert_eq!(next_device(Kind::Output, &a, &s), None);
        assert_eq!(
            view(Kind::Output, &a, &json!({})),
            ("Pick\ndevice".into(), 0)
        );
        assert_eq!(
            view(Kind::Input, &a, &json!({"devices": ["alsa_input.mic"]})),
            ("Desk Mic".into(), 1)
        );
    }

    #[test]
    fn cycling_keys() {
        let mut a = audio();
        let s = json!({"devices": [HEADSET, "alsa_output.unplugged", SPEAKERS, HEADSET]});
        assert_eq!(
            selected(&s),
            vec![HEADSET, "alsa_output.unplugged", SPEAKERS]
        );
        assert_eq!(view(Kind::Output, &a, &s), ("Speakers".into(), 1));
        assert_eq!(next_device(Kind::Output, &a, &s).as_deref(), Some(HEADSET));
        a.default_output = HEADSET.into();
        assert_eq!(
            next_device(Kind::Output, &a, &s).as_deref(),
            Some(SPEAKERS),
            "skips devices that are not connected"
        );
        a.default_output = HDMI.into();
        assert_eq!(
            view(Kind::Output, &a, &s),
            ("HDMI".into(), 0),
            "shows the current device even when it is not in the list"
        );
        assert_eq!(next_device(Kind::Output, &a, &s).as_deref(), Some(HEADSET));
    }

    #[test]
    fn fingerprints_ignore_number_parts() {
        let f = fingerprint;
        assert_eq!(
            f("alsa_output.usb-Logitech_G535-00.analog-stereo"),
            f("alsa_output.usb-Logitech_G535-01.analog-stereo.2")
        );
        assert_eq!(
            f("alsa_input.usb-Generic_USB_Audio_201405280001-00.mono-fallback"),
            f("alsa_input.usb-Generic_USB_Audio_201405280002-00.mono-fallback")
        );
        assert_ne!(
            f("alsa_output.usb-Logitech_G535-00.analog-stereo"),
            f("alsa_output.usb-Logitech_G733-00.analog-stereo")
        );
        assert_ne!(
            f("alsa_output.pci.hdmi-stereo"),
            f("alsa_output.pci.hdmi-stereo-extra1")
        );
        assert_ne!(
            f("alsa_output.headset.analog-stereo"),
            f("alsa_output.headset.iec958-stereo")
        );
    }

    #[test]
    fn matches_a_device_that_came_back_under_a_new_name() {
        let mut a = audio();
        let renamed = "alsa_output.headset.2";
        a.outputs[1].name = renamed.into();
        let s = json!({"devices": [HEADSET]});
        assert_eq!(
            resolve(Kind::Output, &a, &s),
            vec![(HEADSET.to_string(), Some(renamed.to_string()))]
        );
        assert_eq!(next_device(Kind::Output, &a, &s).as_deref(), Some(renamed));
        a.default_output = renamed.into();
        assert_eq!(view(Kind::Output, &a, &s), ("G535\nWireless…".into(), 1));

        let off = json!({"devices": [HEADSET], "matchSimilar": false});
        assert_eq!(
            resolve(Kind::Output, &a, &off),
            vec![(HEADSET.to_string(), None)]
        );
        assert_eq!(view(Kind::Output, &a, &off).1, 0);

        // A Bluetooth headset that comes back with a new profile in its
        // name is matched by the description saved with the key.
        a.outputs[1].name = "bluez_output.AA_BB.headset-head-unit".into();
        let bt = json!({"devices": ["bluez_output.AA_BB.a2dp-sink"],
            "descriptions": {"bluez_output.AA_BB.a2dp-sink": "G535 Wireless Gaming Headset"}});
        assert_eq!(
            next_device(Kind::Output, &a, &bt).as_deref(),
            Some("bluez_output.AA_BB.headset-head-unit")
        );
        assert_eq!(
            next_device(
                Kind::Output,
                &a,
                &json!({"devices": ["bluez_output.AA_BB.a2dp-sink"]})
            ),
            None
        );
    }

    #[test]
    fn a_device_is_matched_once_and_exact_names_win() {
        let mut a = audio();
        a.outputs.push(Device {
            name: "alsa_output.headset.3".into(),
            description: "Other".into(),
        });
        // The headset is connected under its own name, so the missing
        // ".2" entry must not steal it; it gets the ".3" one instead.
        let s = json!({"devices": [HEADSET, "alsa_output.headset.2"]});
        assert_eq!(
            resolve(Kind::Output, &a, &s),
            vec![
                (HEADSET.to_string(), Some(HEADSET.to_string())),
                (
                    "alsa_output.headset.2".to_string(),
                    Some("alsa_output.headset.3".to_string())
                ),
            ]
        );
        let s = json!({"devices": ["alsa_output.headset.2", "alsa_output.headset.4"]});
        let r = resolve(Kind::Output, &a, &s);
        assert_eq!(r[0].1.as_deref(), Some(HEADSET));
        assert_eq!(r[1].1.as_deref(), Some("alsa_output.headset.3"));
    }

    #[test]
    fn moving_apps_is_off_by_default() {
        let (on, keep) = move_settings(&json!({}));
        assert!(!on);
        assert_eq!(keep, vec!["TeamSpeak", "Zoom", "Discord"]);
        let (on, keep) = move_settings(&json!({"moveApps": true, "keepApps": "obs,\n  Mumble , "}));
        assert!(on);
        assert_eq!(keep, vec!["obs", "Mumble"]);
        assert!(move_settings(&json!({"moveApps": true, "keepApps": ""}))
            .1
            .is_empty());
    }

    #[test]
    fn encodes_base64() {
        for (raw, enc) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(raw.as_bytes()), enc);
        }
        for i in ICONS {
            assert!(icon_png(i, true).unwrap().starts_with(b"\x89PNG"));
            assert!(icon_png(i, false).is_some());
        }
        assert_eq!(icon_png("toaster", true), None);
    }

    #[test]
    fn picks_the_icon_of_the_device_shown() {
        let mut a = audio();
        let icons = json!({SPEAKERS: "speakers", HEADSET: "headset", HDMI: "toaster"});
        let one = json!({"devices": [HEADSET], "icons": icons});
        assert_eq!(icon(Kind::Output, &a, &one), Some("headset"));
        let cycle = json!({"devices": [SPEAKERS, HEADSET, HDMI], "icons": icons});
        assert_eq!(icon(Kind::Output, &a, &cycle), Some("speakers"));
        a.default_output = HEADSET.into();
        assert_eq!(icon(Kind::Output, &a, &cycle), Some("headset"));
        a.default_output = HDMI.into();
        assert_eq!(
            icon(Kind::Output, &a, &cycle),
            None,
            "unknown icon names are ignored"
        );
        a.default_output = "alsa_output.other".into();
        assert_eq!(icon(Kind::Output, &a, &cycle), None);
        assert_eq!(icon(Kind::Output, &a, &json!({"devices": [HEADSET]})), None);
        assert_eq!(
            icon(
                Kind::Output,
                &AudioState::with_status(Status::NoServer),
                &one
            ),
            None
        );
    }

    #[test]
    fn parses_action_uuids() {
        assert_eq!(
            kind_from_uuid("com.avonderau.audioswitcher.output"),
            Some(Kind::Output)
        );
        assert_eq!(
            kind_from_uuid("com.avonderau.audioswitcher.input"),
            Some(Kind::Input)
        );
        assert_eq!(kind_from_uuid("com.other.output"), None);
        assert_eq!(kind_from_uuid("com.avonderau.audioswitcher.mute"), None);
    }
}
