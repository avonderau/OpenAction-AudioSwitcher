//! Runs the built plugin binary against a fake Deck host and a fake
//! `pactl`, the way a host launches it.

mod support;

use std::net::TcpListener;
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use support::{FakePactl, HEADSET};
use tungstenite::Message;

#[test]
// The handshake callback's error type is tungstenite's, not ours.
#[allow(clippy::result_large_err)]
fn binary_registers_and_drives_a_key() {
    let fake = FakePactl::start();
    let host = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = host.local_addr().unwrap().port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_audioswitcher"))
        .env("AUDIOSWITCHER_PACTL", &fake.path)
        .args([
            "-port",
            &port.to_string(),
            "-pluginUUID",
            "tok123",
            "-registerEvent",
            "registerPlugin",
            "-info",
            "{}",
        ])
        .spawn()
        .unwrap();

    let (stream, _) = host.accept().unwrap();
    let mut origin = None;
    let mut ws = tungstenite::accept_hdr(
        stream,
        |req: &tungstenite::handshake::server::Request, resp| {
            origin = req.headers().get("origin").cloned();
            Ok(resp)
        },
    )
    .unwrap();
    assert!(
        origin.is_none(),
        "the plugin must not send an Origin header"
    );
    ws.get_ref()
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();

    let mut seen: Vec<Value> = Vec::new();
    let mut wait =
        |ws: &mut tungstenite::WebSocket<_>, what: &str, pred: &dyn Fn(&Value) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(m) = seen.iter().find(|m| pred(m)) {
                    return m.clone();
                }
                assert!(Instant::now() < deadline, "no {what}; got {seen:#?}");
                if let Ok(Message::Text(t)) = ws.read() {
                    seen.push(serde_json::from_str(&t).unwrap());
                }
            }
        };
    let send = |ws: &mut tungstenite::WebSocket<_>, v: Value| {
        ws.send(Message::Text(v.to_string())).unwrap()
    };

    wait(&mut ws, "registration", &|m| {
        m["event"] == "registerPlugin" && m["uuid"] == "tok123"
    });
    send(
        &mut ws,
        json!({"event": "willAppear", "action": "com.avonderau.audioswitcher.output", "context": "c1", "payload": {"settings": {"devices": [HEADSET], "labels": {HEADSET: "Headset"}}}}),
    );
    let state = |want: u8| {
        move |m: &Value| {
            m["event"] == "setState" && m["context"] == "c1" && m["payload"]["state"] == want
        }
    };
    wait(&mut ws, "title", &|m| {
        m["event"] == "setTitle" && m["context"] == "c1" && m["payload"]["title"] == "Headset"
    });
    wait(&mut ws, "inactive", &state(0));
    send(&mut ws, json!({"event": "keyDown", "context": "c1"}));
    wait(&mut ws, "active", &state(1));
    assert_eq!(fake.read("default_sink"), HEADSET);

    ws.close(None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let _ = ws.read();
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "plugin exited with {status}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "plugin did not exit after the host closed"
        );
    }
}
