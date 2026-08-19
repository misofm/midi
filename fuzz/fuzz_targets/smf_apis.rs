#![no_main]

use libfuzzer_sys::fuzz_target;
use miso_midi_core::{
    ScoreParseLimits, ScoreParseMode, ScoreParseOptions, parse_score_smf,
    parse_score_smf_with_limits, parse_score_smf_with_options, parse_smf, scan_smf,
};

const MAX_FUZZ_INPUT_BYTES: usize = 64 * 1024;

const SMALL_LIMITS: ScoreParseLimits = ScoreParseLimits {
    max_input_bytes: 16 * 1024,
    max_source_tracks: 4,
    max_track_bytes: 4 * 1024,
    max_events: 1_024,
    max_note_starts: 256,
    max_text_bytes: 1_024,
};

const FINITE_COMPATIBLE: ScoreParseOptions = ScoreParseOptions {
    limits: SMALL_LIMITS,
    mode: ScoreParseMode::Compatible,
};

const FINITE_STRICT: ScoreParseOptions = ScoreParseOptions {
    limits: SMALL_LIMITS,
    mode: ScoreParseMode::Strict,
};

fuzz_target!(|data: &[u8]| {
    // The trusted score parser deliberately has no resource policy. Cap its
    // input here so one fuzzer worker cannot retain an unbounded score.
    let data = &data[..data.len().min(MAX_FUZZ_INPUT_BYTES)];

    let _ = scan_smf(data);
    let _ = parse_smf(data);
    let _ = parse_score_smf(data);
    let _ = parse_score_smf_with_limits(data, SMALL_LIMITS);
    let _ = parse_score_smf_with_options(data, FINITE_COMPATIBLE);
    let _ = parse_score_smf_with_options(data, FINITE_STRICT);
});
