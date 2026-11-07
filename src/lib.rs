//! Audio output and input switching keys for Stream Deck-compatible hosts
//! (OpenLinkHub Nexus, OpenDeck, Elgato Stream Deck), talking to PulseAudio
//! or PipeWire through `pactl`.

pub mod deck;
pub mod pactl;
pub mod plugin;
