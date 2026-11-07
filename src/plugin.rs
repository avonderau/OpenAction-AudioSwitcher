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

/// What a key shows for a device: its label from the key settings, else
/// the device description, else the raw name.
fn label(kind: Kind, audio: &AudioState, settings: &Value, name: &str) -> String {
    let custom = settings
        .get("labels")
        .and_then(|l| l.get(name))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if !custom.is_empty() {
        return custom.to_string();
    }
    audio
        .devices(kind)
        .iter()
        .find(|d| d.name == name)
        .map(|d| d.description.clone())
        .unwrap_or_else(|| name.to_string())
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
    let names = selected(settings);
    let current = audio.default_device(kind);
    match names.as_slice() {
        [] => ("Pick\ndevice".into(), 0),
        [one] => (
            fit(&label(kind, audio, settings, one)),
            u8::from(one == current),
        ),
        _ => (
            fit(&label(kind, audio, settings, current)),
            u8::from(names.iter().any(|n| n == current)),
        ),
    }
}

/// The device a press switches to, or None when none of the key's devices
/// is connected.
pub fn next_device(kind: Kind, audio: &AudioState, settings: &Value) -> Option<String> {
    let present = |n: &String| audio.devices(kind).iter().any(|d| &d.name == n);
    let names: Vec<String> = selected(settings).into_iter().filter(present).collect();
    let current = audio.default_device(kind);
    let next = match names.iter().position(|n| n == current) {
        Some(i) => (i + 1) % names.len(),
        None => 0,
    };
    names.into_iter().nth(next)
}

struct Key {
    kind: Kind,
    settings: Value,
    shown: Option<(String, u8)>,
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
            Some(name) => self.request(Command::SetDefault { kind, name }, Some(context)),
            None => self.alert(context),
        }
    }

    fn send_devices(&mut self, context: &str) {
        let action = self.inspectors.get(context).cloned().unwrap_or_default();
        let Some(kind) = kind_from_uuid(&action) else {
            return;
        };
        self.outbox.push(json!({"event": "sendToPropertyInspector", "context": context, "action": action, "payload": {
            "type": "devices",
            "status": self.audio.status,
            "kind": kind,
            "devices": self.audio.devices(kind),
            "current": self.audio.default_device(kind),
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
