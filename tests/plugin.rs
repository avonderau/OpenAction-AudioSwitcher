mod support;

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use audioswitcher::pactl::{spawn, AudioEvent};
use audioswitcher::plugin::Plugin;
use serde_json::{json, Value};
use support::{FakePactl, BUILTIN_MIC, HEADSET, SPEAKERS, USB_MIC};

const OUTPUT: &str = "com.avonderau.audioswitcher.output";
const INPUT: &str = "com.avonderau.audioswitcher.input";

/// Runs a Plugin against the real worker and records what it sends.
struct Harness {
    plugin: Plugin,
    events: Receiver<AudioEvent>,
    sent: Vec<Value>,
}

impl Harness {
    fn new(pactl: &str) -> Harness {
        let (tx, rx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        spawn(pactl.to_string(), rx, move |e| {
            let _ = etx.send(e);
        });
        Harness {
            plugin: Plugin::new(tx),
            events: erx,
            sent: Vec::new(),
        }
    }

    fn flush(&mut self) {
        self.sent.append(&mut self.plugin.outbox);
    }

    fn deck(&mut self, msg: Value) {
        self.plugin.on_deck(&msg);
        self.flush();
    }

    fn appear(&mut self, action: &str, context: &str, settings: Value) {
        self.deck(json!({"event": "willAppear", "action": action, "context": context, "payload": {"settings": settings}}));
    }

    fn press(&mut self, context: &str) {
        self.deck(json!({"event": "keyDown", "context": context}));
        self.deck(json!({"event": "keyUp", "context": context}));
    }

    fn last(&self, event: &str, context: &str) -> Option<&Value> {
        self.sent
            .iter()
            .rev()
            .find(|m| m["event"] == event && m["context"] == context)
            .map(|m| &m["payload"])
    }

    fn title(&self, context: &str) -> String {
        self.last("setTitle", context)
            .and_then(|p| p["title"].as_str())
            .unwrap_or("")
            .to_string()
    }

    fn state(&self, context: &str) -> i64 {
        self.last("setState", context)
            .and_then(|p| p["state"].as_i64())
            .unwrap_or(-1)
    }

    fn alerts(&self, context: &str) -> usize {
        self.sent
            .iter()
            .filter(|m| m["event"] == "showAlert" && m["context"] == context)
            .count()
    }

    /// Feeds audio events until `pred` holds.
    fn until(&mut self, what: &str, pred: impl Fn(&Harness) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while !pred(self) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; sent: {:#?}",
                self.sent
            );
            if let Ok(e) = self.events.recv_timeout(Duration::from_millis(50)) {
                self.plugin.on_audio(e);
                self.flush();
            }
        }
    }
}

#[test]
fn keys_switch_devices_and_follow_the_server() {
    let fake = FakePactl::start();
    let mut h = Harness::new(&fake.path);

    h.appear(
        OUTPUT,
        "k-head",
        json!({"devices": [HEADSET], "labels": {HEADSET: "Headset"}}),
    );
    assert_eq!(
        h.title("k-head"),
        "Audio\n…",
        "nothing shown before the first state"
    );
    h.appear(OUTPUT, "k-cycle", json!({"devices": [SPEAKERS, HEADSET]}));
    h.appear(INPUT, "k-mic", json!({"devices": [USB_MIC]}));
    h.until("first state", |h| h.title("k-head") == "Headset");
    assert_eq!(h.state("k-head"), 0);
    assert_eq!(h.title("k-cycle"), "Built-in\nAudio…");
    assert_eq!(h.state("k-cycle"), 1);
    assert_eq!((h.title("k-mic").as_str(), h.state("k-mic")), ("Yeti", 0));

    h.press("k-head");
    h.until("headset selected", |h| h.state("k-head") == 1);
    assert_eq!(fake.read("default_sink"), HEADSET);
    assert_eq!(h.title("k-cycle"), "G535\nHeadset");

    h.press("k-cycle");
    h.until("cycled to speakers", |h| h.state("k-head") == 0);
    assert_eq!(fake.read("default_sink"), SPEAKERS);
    assert_eq!(h.title("k-cycle"), "Built-in\nAudio…");

    h.press("k-mic");
    h.until("USB mic selected", |h| h.state("k-mic") == 1);
    assert_eq!(fake.read("default_source"), USB_MIC);

    // A change made outside the Deck, e.g. in the desktop sound settings.
    fake.write("default_source", BUILTIN_MIC);
    fake.notify();
    h.until("mic follows the server", |h| h.state("k-mic") == 0);

    // A key whose device is unplugged alerts and does not call pactl.
    fake.write(
        "sinks",
        &format!("{SPEAKERS}|Built-in Audio Analog Stereo\n"),
    );
    fake.notify();
    h.appear(OUTPUT, "k-raw", json!({"devices": [HEADSET]}));
    assert_eq!(h.title("k-raw"), "G535\nHeadset");
    h.until("headset removed", |h| h.title("k-raw") == "alsa_outp…");
    let calls = fake.calls().len();
    h.press("k-head");
    assert_eq!(h.alerts("k-head"), 1);
    assert_eq!(h.title("k-head"), "Headset");
    let set_calls = fake.calls()[calls..]
        .iter()
        .filter(|c| c.starts_with("set-default"))
        .count();
    assert_eq!(set_calls, 0);
}

#[test]
fn single_device_key_on_the_current_device_stays_put() {
    let fake = FakePactl::start();
    let mut h = Harness::new(&fake.path);
    h.appear(OUTPUT, "k", json!({"devices": [SPEAKERS]}));
    h.until("ready", |h| h.state("k") == 1);
    h.press("k");
    h.until("refresh after press", |_| {
        fake.calls()
            .iter()
            .any(|c| c == &format!("set-default-sink {SPEAKERS}"))
    });
    assert_eq!(h.alerts("k"), 0);
    assert_eq!(h.state("k"), 1);
}

#[test]
fn failed_switch_alerts_the_key() {
    let fake = FakePactl::start();
    let mut h = Harness::new(&fake.path);
    h.appear(OUTPUT, "k", json!({"devices": [HEADSET]}));
    h.until("ready", |h| h.title("k") == "G535\nHeadset");
    // The headset goes away before the plugin hears about it.
    fake.write("sinks", &format!("{SPEAKERS}|Speakers\n"));
    h.press("k");
    h.until("alert", |h| h.alerts("k") == 1);
    assert!(h.sent.iter().any(|m| m["event"] == "logMessage"
        && m["payload"]["message"]
            .as_str()
            .is_some_and(|s| s.contains("No such entity"))));
}

#[test]
fn shows_when_the_audio_server_is_missing() {
    let fake = FakePactl::start();
    fake.set_server_down(true);
    let mut h = Harness::new(&fake.path);
    h.appear(OUTPUT, "k", json!({"devices": [SPEAKERS]}));
    h.until("no server", |h| h.title("k") == "No audio\nserver");
    h.press("k");
    assert_eq!(h.alerts("k"), 1);

    fake.set_server_down(false);
    h.until("server back", |h| h.title("k") == "Built-in\nAudio…");

    let mut h = Harness::new("/nonexistent/pactl");
    h.appear(INPUT, "k", json!({}));
    h.until("no pactl", |h| h.title("k") == "Install\npactl");
}

#[test]
fn settings_page_gets_the_device_list() {
    let fake = FakePactl::start();
    let mut h = Harness::new(&fake.path);
    h.appear(INPUT, "k", json!({}));
    h.until("ready", |h| h.title("k") == "Pick\ndevice");
    h.deck(json!({"event": "propertyInspectorDidAppear", "action": INPUT, "context": "k"}));
    let p = h.last("sendToPropertyInspector", "k").unwrap().clone();
    assert_eq!(p["type"], "devices");
    assert_eq!(p["status"], "ready");
    assert_eq!(p["kind"], "input");
    assert_eq!(p["current"], BUILTIN_MIC);
    assert_eq!(
        p["devices"],
        json!([
            {"name": BUILTIN_MIC, "description": "Built-in Audio Analog Stereo"},
            {"name": USB_MIC, "description": "Yeti"},
        ])
    );

    // Open pages hear about changes.
    fake.write("default_source", USB_MIC);
    fake.notify();
    h.until("page update", |h| {
        h.last("sendToPropertyInspector", "k").unwrap()["current"] == USB_MIC
    });

    h.deck(json!({"event": "didReceiveSettings", "action": INPUT, "context": "k", "payload": {"settings": {"devices": [USB_MIC]}}}));
    assert_eq!((h.title("k").as_str(), h.state("k")), ("Yeti", 1));
}

#[test]
fn replugged_device_with_a_new_name_is_matched() {
    let fake = FakePactl::start();
    let mut h = Harness::new(&fake.path);
    h.appear(OUTPUT, "k", json!({"devices": [HEADSET]}));
    h.until("ready", |h| h.title("k") == "G535\nHeadset");

    let renamed = "alsa_output.usb-Logitech_G535-01.analog-stereo.2";
    fake.write(
        "sinks",
        &format!("{SPEAKERS}|Built-in Audio Analog Stereo\n{renamed}|G535 Headset\n"),
    );
    fake.notify();
    h.deck(json!({"event": "propertyInspectorDidAppear", "action": OUTPUT, "context": "k"}));
    h.until("page sees the renamed headset", |h| {
        h.last("sendToPropertyInspector", "k").unwrap()["matches"][HEADSET] == renamed
    });
    h.press("k");
    h.until("renamed headset selected", |h| h.state("k") == 1);
    assert_eq!(fake.read("default_sink"), renamed);
}
