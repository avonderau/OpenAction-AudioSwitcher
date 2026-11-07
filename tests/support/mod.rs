//! A fake `pactl` for tests: a shell script that keeps its devices and
//! defaults in files, prints the same text as `pactl` with `LC_ALL=C`, and
//! reports changes through `pactl subscribe`.
#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const SPEAKERS: &str = "alsa_output.pci-0000_00_1f.3.analog-stereo";
pub const HEADSET: &str = "alsa_output.usb-Logitech_G535-00.analog-stereo";
pub const BUILTIN_MIC: &str = "alsa_input.pci-0000_00_1f.3.analog-stereo";
pub const USB_MIC: &str = "alsa_input.usb-Blue_Yeti-00.analog-stereo";

const SCRIPT: &str = r#"#!/bin/sh
d='@DIR@'
echo "$*" >> "$d/calls"
if [ -e "$d/down" ]; then
  echo 'Connection failure: Connection refused' >&2
  exit 1
fi
list() {
  n=0
  while IFS='|' read -r name desc; do
    n=$((n + 1))
    printf '%s #%d\n\tState: SUSPENDED\n\tName: %s\n\tDescription: %s\n\n' "$2" "$n" "$name" "$desc"
  done < "$d/$1"
}
case "$1" in
  info)
    printf 'Server Name: fake\nDefault Sink: %s\nDefault Source: %s\n' "$(cat "$d/default_sink")" "$(cat "$d/default_source")" ;;
  list)
    case "$2" in
      sinks) list sinks Sink ;;
      sources) list sources Source ;;
    esac ;;
  set-default-sink|set-default-source)
    f=default_sink
    [ "$1" = set-default-source ] && f=default_source
    if ! cut -d'|' -f1 "$d/${f#default_}s" | grep -qxF "$2"; then
      echo 'Failure: No such entity' >&2
      exit 1
    fi
    printf '%s' "$2" > "$d/$f"
    touch "$d/changed" ;;
  subscribe)
    while [ ! -e "$d/down" ]; do
      if [ -e "$d/changed" ]; then
        rm -f "$d/changed"
        echo "Event 'change' on server #0"
      fi
      sleep 0.05
    done ;;
esac
"#;

static NEXT: AtomicUsize = AtomicUsize::new(0);

pub struct FakePactl {
    pub dir: PathBuf,
    pub path: String,
}

impl FakePactl {
    /// Speakers and a headset, a built-in and a USB mic; the speakers and
    /// the built-in mic are the defaults.
    pub fn start() -> FakePactl {
        let dir = std::env::temp_dir().join(format!(
            "audioswitcher-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let script = dir.join("pactl");
        fs::write(&script, SCRIPT.replace("@DIR@", dir.to_str().unwrap())).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let fake = FakePactl {
            path: script.to_str().unwrap().to_string(),
            dir,
        };
        fake.write(
            "sinks",
            &format!("{SPEAKERS}|Built-in Audio Analog Stereo\n{HEADSET}|G535 Headset\n"),
        );
        fake.write(
            "sources",
            &format!("{BUILTIN_MIC}|Built-in Audio Analog Stereo\n{USB_MIC}|Yeti\n"),
        );
        fake.write("default_sink", SPEAKERS);
        fake.write("default_source", BUILTIN_MIC);
        fake
    }

    pub fn write(&self, file: &str, text: &str) {
        fs::write(self.dir.join(file), text).unwrap();
    }

    /// Tells `pactl subscribe` listeners that something changed.
    pub fn notify(&self) {
        self.write("changed", "");
    }

    pub fn set_server_down(&self, down: bool) {
        if down {
            self.write("down", "");
        } else {
            let _ = fs::remove_file(self.dir.join("down"));
        }
    }

    pub fn read(&self, file: &str) -> String {
        fs::read_to_string(self.dir.join(file)).unwrap_or_default()
    }

    pub fn calls(&self) -> Vec<String> {
        self.read("calls").lines().map(str::to_string).collect()
    }
}

impl Drop for FakePactl {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
