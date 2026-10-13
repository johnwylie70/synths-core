//! Voice changers, as data: each is a short list of parameter values that
//! turns the live input (or any part) into a character. They are handed
//! out with the parameter table (`voice_presets`), so a host need not type
//! them in again.

use crate::engine::Engine;
use crate::params;

/// One voice changer.
pub struct VoicePreset {
    pub name: &'static str,
    pub help: &'static str,
    /// Parameter names and values. Names of part parameters are for the
    /// part the voice plays through; the others are for the whole engine.
    pub values: &'static [(&'static str, f32)],
}

pub const VOICE_PRESETS: &[VoicePreset] = &[
    VoicePreset {
        name: "robot",
        help: "A machine: the voice multiplied by a 50 Hz wave, ringing in a metal pipe.",
        values: &[
            ("fx.ring", 1.0),
            ("fx.ring_freq", 50.0),
            ("fx.comb", 0.45),
            ("fx.comb_freq", 110.0),
            ("fx.comb_feedback", 0.8),
            ("fx.bits", 10.0),
        ],
    },
    VoicePreset {
        name: "chipmunk",
        help: "Small and squeaky: everything seven semitones higher.",
        values: &[("fx.pitch", 7.0)],
    },
    VoicePreset {
        name: "monster",
        help: "Huge and deep: seven semitones lower, with a growl and a little room.",
        values: &[("fx.pitch", -7.0), ("fx.drive", 0.25), ("send.reverb", 0.25), ("reverb.size", 0.7), ("reverb.damping", 0.6)],
    },
    VoicePreset {
        name: "cave",
        help: "Far inside a cave: long echoes and a very big room.",
        values: &[
            ("send.delay", 0.5),
            ("send.reverb", 0.7),
            ("delay.time", 0.38),
            ("delay.feedback", 0.55),
            ("delay.tone", 2500.0),
            ("reverb.size", 0.92),
            ("reverb.damping", 0.3),
        ],
    },
    VoicePreset {
        name: "telephone",
        help: "Down an old phone line: only the middle of the voice, coarse and a little crushed.",
        values: &[
            ("filter.type", 3.0),
            ("filter.slope", 1.0),
            ("filter.cutoff", 1200.0),
            ("filter.resonance", 0.2),
            ("fx.drive", 0.2),
            ("fx.bits", 8.0),
            ("fx.downsample", 6.0),
        ],
    },
    VoicePreset {
        name: "alien",
        help: "Not from here: higher, multiplied by a 180 Hz wave, with a slow metallic sweep.",
        values: &[
            ("fx.pitch", 4.0),
            ("fx.ring", 0.8),
            ("fx.ring_freq", 180.0),
            ("fx.comb", 0.4),
            ("fx.comb_freq", 600.0),
            ("fx.comb_feedback", 0.75),
            ("fx.comb_rate", 0.4),
            ("fx.comb_depth", 0.5),
            ("send.chorus", 0.4),
        ],
    },
    VoicePreset {
        name: "whisper",
        help: "Hoarse and breathy: the voice's pitch blurred into noise, with its low end taken off.",
        values: &[
            ("filter.type", 2.0),
            ("filter.cutoff", 500.0),
            ("filter.resonance", 0.0),
            ("fx.ring", 1.0),
            ("fx.ring_wave", 1.0),
            ("fx.ring_freq", 350.0),
        ],
    },
];

/// Look a voice changer up by name.
pub fn voice_preset(name: &str) -> Option<&'static VoicePreset> {
    VOICE_PRESETS.iter().find(|p| p.name == name)
}

/// Make `part` (and the engine's echo and reverb) sound like a voice
/// changer: every parameter that any voice changer sets goes back to its
/// default, and then this one's values are set. `None` only does the first
/// half, which is the plain voice. Not for the middle of a render.
pub fn apply_voice_preset(engine: &mut Engine, part: u16, preset: Option<&VoicePreset>) {
    for other in VOICE_PRESETS {
        for (name, _) in other.values {
            if let Some(info) = params::by_name(name) {
                engine.set_param(part, info.id as usize, info.default);
            }
        }
    }
    for (name, value) in preset.map_or(&[][..], |p| p.values) {
        if let Some(info) = params::by_name(name) {
            engine.set_param(part, info.id as usize, *value);
        }
    }
}

/// A voice changer as a row of the table in `docs/engine-api.md`, which is
/// made by `cargo run --example param_table presets`; a test checks the
/// two agree.
pub fn doc_row(p: &VoicePreset) -> String {
    let values: Vec<String> = p.values.iter().map(|(name, value)| format!("`{name}` {value}")).collect();
    format!("| `{}` | {} | {} |", p.name, values.join(", "), p.help)
}
