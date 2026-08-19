#![no_main]

use libfuzzer_sys::fuzz_target;
use miso_midi_core::{
    ScoreParseLimits, ScoreParseMode, ScoreParseOptions, parse_score_smf,
    parse_score_smf_with_limits, parse_score_smf_with_options, parse_smf, scan_smf,
};

const MAX_SOURCE_BYTES: usize = 192;
const MAX_NOTES: usize = 32;

const FINITE_LIMITS: ScoreParseLimits = ScoreParseLimits {
    max_input_bytes: 4 * 1024,
    max_source_tracks: 4,
    max_track_bytes: 2 * 1024,
    max_events: 256,
    max_note_starts: 64,
    max_text_bytes: 256,
};

const FINITE_COMPATIBLE: ScoreParseOptions = ScoreParseOptions {
    limits: FINITE_LIMITS,
    mode: ScoreParseMode::Compatible,
};

const FINITE_STRICT: ScoreParseOptions = ScoreParseOptions {
    limits: FINITE_LIMITS,
    mode: ScoreParseMode::Strict,
};

fuzz_target!(|data: &[u8]| {
    let smf = build_smf(&data[..data.len().min(MAX_SOURCE_BYTES)]);

    assert!(scan_smf(&smf).is_ok(), "generator must produce a valid SMF");
    assert!(
        parse_smf(&smf).is_ok(),
        "generator must produce a parsable SMF"
    );

    let trusted = parse_score_smf(&smf).expect("generator must produce a score");
    let unlimited = parse_score_smf_with_limits(&smf, ScoreParseLimits::UNLIMITED)
        .expect("unlimited checked parser must accept generated SMF");
    let compatible = parse_score_smf_with_options(&smf, FINITE_COMPATIBLE)
        .expect("finite compatible parser must accept generated SMF");
    let strict = parse_score_smf_with_options(&smf, FINITE_STRICT)
        .expect("finite strict parser must accept generated SMF");

    assert_eq!(trusted, unlimited);
    assert_eq!(trusted, compatible);
    assert_eq!(trusted, strict);
});

fn build_smf(source: &[u8]) -> Vec<u8> {
    let mut track = Vec::with_capacity(8 + source.len() * 4);

    // A track name drives the text-normalization path while retaining valid
    // framing regardless of the fuzzer-provided name byte.
    track.extend_from_slice(&[0, 0xff, 0x03, 1, source.first().copied().unwrap_or(b'f')]);

    for bytes in source.chunks(3).take(MAX_NOTES) {
        let pitch = bytes.first().copied().unwrap_or(60) & 0x7f;
        let velocity = (bytes.get(1).copied().unwrap_or(100) & 0x7f).max(1);
        let duration = bytes.get(2).copied().unwrap_or(1) & 0x7f;
        let channel = pitch & 1;

        // Program changes and channel-separated notes exercise score grouping
        // while all deltas and lengths remain single-byte valid VLQs.
        track.extend_from_slice(&[0, 0xc0 | channel, pitch]);
        track.extend_from_slice(&[0, 0x90 | channel, pitch, velocity]);
        track.extend_from_slice(&[duration, 0x80 | channel, pitch, 0]);
    }

    track.extend_from_slice(&[0, 0xff, 0x2f, 0]);

    let track_len = u32::try_from(track.len()).expect("bounded track length fits u32");
    let mut smf = Vec::with_capacity(22 + track.len());
    smf.extend_from_slice(b"MThd");
    smf.extend_from_slice(&6_u32.to_be_bytes());
    smf.extend_from_slice(&0_u16.to_be_bytes());
    smf.extend_from_slice(&1_u16.to_be_bytes());
    smf.extend_from_slice(&96_u16.to_be_bytes());
    smf.extend_from_slice(b"MTrk");
    smf.extend_from_slice(&track_len.to_be_bytes());
    smf.extend_from_slice(&track);
    smf
}
