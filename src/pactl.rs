//! The audio server: PulseAudio, or PipeWire through pipewire-pulse, driven
//! with the `pactl` command.
//!
//! [`spawn`] runs one worker thread. It keeps `pactl subscribe` running to
//! hear about device changes, reports an [`AudioState`] whenever the device
//! lists or the defaults change, and carries out [`Request`]s. Failures come
//! back as [`AudioEvent::Failed`] so the key that asked can show an alert.

use std::io::{self, BufRead, BufReader};
use std::process::{Child, Command as Process, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

const POLL: Duration = Duration::from_millis(100);
const RETRY: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A sink: speakers, headphones, HDMI.
    Output,
    /// A source: microphones and line inputs (monitors are left out).
    Input,
}

impl Kind {
    fn list_args(self) -> [&'static str; 2] {
        match self {
            Kind::Output => ["list", "sinks"],
            Kind::Input => ["list", "sources"],
        }
    }

    fn set_default(self) -> &'static str {
        match self {
            Kind::Output => "set-default-sink",
            Kind::Input => "set-default-source",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Device {
    /// The stable device name, e.g. `alsa_output.usb-Foo-00.analog-stereo`.
    pub name: String,
    /// The human-readable name, e.g. `Foo Headset Analog Stereo`.
    pub description: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Starting,
    Ready,
    /// `pactl` runs but cannot reach PulseAudio or pipewire-pulse.
    NoServer,
    /// `pactl` is not installed.
    NoPactl,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioState {
    pub status: Status,
    pub outputs: Vec<Device>,
    pub inputs: Vec<Device>,
    pub default_output: String,
    pub default_input: String,
}

impl AudioState {
    pub fn with_status(status: Status) -> AudioState {
        AudioState {
            status,
            ..AudioState::default()
        }
    }

    pub fn devices(&self, kind: Kind) -> &[Device] {
        match kind {
            Kind::Output => &self.outputs,
            Kind::Input => &self.inputs,
        }
    }

    pub fn default_device(&self, kind: Kind) -> &str {
        match kind {
            Kind::Output => &self.default_output,
            Kind::Input => &self.default_input,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    SetDefault { kind: Kind, name: String },
    Refresh,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub command: Command,
    /// The key that asked, so a failure can be shown on it.
    pub context: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioEvent {
    State(AudioState),
    Failed {
        context: Option<String>,
        error: String,
    },
    Log(String),
}

/// Reads the devices from `pactl list sinks` or `pactl list sources`
/// output (run with `LC_ALL=C`). Monitor sources are skipped.
pub fn parse_devices(text: &str) -> Vec<Device> {
    let mut devices = Vec::new();
    let mut current: Option<Device> = None;
    for line in text.lines() {
        if line.starts_with("Sink #") || line.starts_with("Source #") {
            devices.extend(current.take());
            current = Some(Device {
                name: String::new(),
                description: String::new(),
            });
            continue;
        }
        let Some(dev) = current.as_mut() else {
            continue;
        };
        let line = line.trim_start();
        if let Some(v) = line.strip_prefix("Name: ") {
            dev.name = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Description: ") {
            dev.description = v.trim().to_string();
        }
    }
    devices.extend(current);
    devices.retain(|d| !d.name.is_empty() && !d.name.ends_with(".monitor"));
    for d in &mut devices {
        if d.description.is_empty() {
            d.description = d.name.clone();
        }
    }
    devices
}

/// Reads the default sink and source from `pactl info` output.
pub fn parse_defaults(text: &str) -> (String, String) {
    let mut sink = String::new();
    let mut source = String::new();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("Default Sink: ") {
            sink = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Default Source: ") {
            source = v.trim().to_string();
        }
    }
    (sink, source)
}

/// Whether a `pactl subscribe` line can change what the keys show.
fn relevant(line: &str) -> bool {
    [" on server", " on sink #", " on source #", " on card #"]
        .iter()
        .any(|s| line.contains(s))
}

enum PactlError {
    Missing,
    Failed(String),
}

/// Starts the worker. `pactl` is the command to run (normally "pactl").
/// The worker stops when `requests` closes.
pub fn spawn<F>(pactl: String, requests: Receiver<Request>, emit: F) -> thread::JoinHandle<()>
where
    F: Fn(AudioEvent) + Send + 'static,
{
    thread::spawn(move || {
        Worker {
            pactl,
            requests,
            emit: Box::new(emit),
            state: AudioState::default(),
        }
        .run()
    })
}

/// A running `pactl subscribe`. `changes` gets a message for each relevant
/// event and disconnects when the process ends.
struct Watch {
    child: Child,
    changes: Receiver<()>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Worker {
    pactl: String,
    requests: Receiver<Request>,
    emit: Box<dyn Fn(AudioEvent) + Send>,
    state: AudioState,
}

impl Worker {
    fn run(mut self) {
        let mut watch: Option<Watch> = None;
        let mut retry_at = Instant::now();
        loop {
            if watch.is_none() && Instant::now() >= retry_at {
                watch = self.watch();
                self.refresh();
                retry_at = Instant::now() + RETRY;
            }
            match self.requests.recv_timeout(POLL) {
                Ok(req) => self.handle(req),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            let Some(w) = &watch else { continue };
            let (mut changed, mut ended) = (false, false);
            loop {
                match w.changes.try_recv() {
                    Ok(()) => changed = true,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        ended = true;
                        break;
                    }
                }
            }
            if ended {
                watch = None;
                retry_at = Instant::now() + RETRY;
            }
            if changed || ended {
                self.refresh();
            }
        }
    }

    fn watch(&self) -> Option<Watch> {
        let mut child = Process::new(&self.pactl)
            .arg("subscribe")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdout = child.stdout.take()?;
        let (tx, changes) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                if relevant(&line) && tx.send(()).is_err() {
                    return;
                }
            }
        });
        Some(Watch { child, changes })
    }

    fn pactl(&self, args: &[&str]) -> Result<String, PactlError> {
        let out = Process::new(&self.pactl)
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => PactlError::Missing,
                _ => PactlError::Failed(format!("{}: {e}", self.pactl)),
            })?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(PactlError::Failed(format!(
                "pactl {}: {}",
                args.join(" "),
                if err.is_empty() {
                    out.status.to_string()
                } else {
                    err
                }
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn read_state(&self) -> Result<AudioState, PactlError> {
        let (default_output, default_input) = parse_defaults(&self.pactl(&["info"])?);
        Ok(AudioState {
            status: Status::Ready,
            outputs: parse_devices(&self.pactl(&Kind::Output.list_args())?),
            inputs: parse_devices(&self.pactl(&Kind::Input.list_args())?),
            default_output,
            default_input,
        })
    }

    fn refresh(&mut self) {
        let next = match self.read_state() {
            Ok(s) => s,
            Err(PactlError::Missing) => AudioState::with_status(Status::NoPactl),
            Err(PactlError::Failed(e)) => {
                if self.state.status != Status::NoServer {
                    (self.emit)(AudioEvent::Log(e));
                }
                AudioState::with_status(Status::NoServer)
            }
        };
        if next != self.state {
            self.state = next.clone();
            (self.emit)(AudioEvent::State(next));
        }
    }

    fn handle(&mut self, req: Request) {
        if let Command::SetDefault { kind, name } = &req.command {
            let result = self.pactl(&[kind.set_default(), name]);
            let error = match result {
                Ok(_) => None,
                Err(PactlError::Missing) => Some("pactl is not installed".to_string()),
                Err(PactlError::Failed(e)) => Some(e),
            };
            if let Some(error) = error {
                (self.emit)(AudioEvent::Failed {
                    context: req.context,
                    error,
                });
            }
        }
        self.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINKS: &str = "Sink #47
\tState: SUSPENDED
\tName: alsa_output.pci-0000_00_1f.3.analog-stereo
\tDescription: Built-in Audio Analog Stereo
\tDriver: PipeWire
\tProperties:
\t\tdevice.description = \"Built-in Audio\"

Sink #52
\tState: RUNNING
\tName: alsa_output.usb-Logitech_G535-00.analog-stereo
\tDescription: G535 Wireless Gaming Headset Analog Stereo
";

    const SOURCES: &str = "Source #48
\tName: alsa_output.pci-0000_00_1f.3.analog-stereo.monitor
\tDescription: Monitor of Built-in Audio Analog Stereo
Source #49
\tName: alsa_input.pci-0000_00_1f.3.analog-stereo
\tDescription: Built-in Audio Analog Stereo
Source #50
\tName: no_description_source
";

    #[test]
    fn parses_device_lists() {
        assert_eq!(
            parse_devices(SINKS),
            vec![
                Device {
                    name: "alsa_output.pci-0000_00_1f.3.analog-stereo".into(),
                    description: "Built-in Audio Analog Stereo".into(),
                },
                Device {
                    name: "alsa_output.usb-Logitech_G535-00.analog-stereo".into(),
                    description: "G535 Wireless Gaming Headset Analog Stereo".into(),
                },
            ]
        );
        let sources = parse_devices(SOURCES);
        assert_eq!(sources.len(), 2, "monitor sources are skipped");
        assert_eq!(sources[1].description, "no_description_source");
        assert!(parse_devices("").is_empty());
    }

    #[test]
    fn parses_defaults() {
        let info = "Server String: /run/user/1000/pulse/native
Server Name: PulseAudio (on PipeWire 1.0.5)
Default Sink: alsa_output.usb-Logitech_G535-00.analog-stereo
Default Source: alsa_input.pci-0000_00_1f.3.analog-stereo
Cookie: 1234:abcd
";
        assert_eq!(
            parse_defaults(info),
            (
                "alsa_output.usb-Logitech_G535-00.analog-stereo".into(),
                "alsa_input.pci-0000_00_1f.3.analog-stereo".into()
            )
        );
    }

    #[test]
    fn filters_subscribe_events() {
        assert!(relevant("Event 'change' on server #0"));
        assert!(relevant("Event 'new' on sink #61"));
        assert!(relevant("Event 'remove' on source #62"));
        assert!(relevant("Event 'change' on card #40"));
        assert!(!relevant("Event 'change' on sink-input #90"));
        assert!(!relevant("Event 'new' on source-output #91"));
        assert!(!relevant("Event 'change' on client #12"));
    }
}
