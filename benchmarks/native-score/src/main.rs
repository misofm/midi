//! Reproducible native score-parser and diagnostic-floor measurements.
//!
//! This is a standalone workspace package: it has no production dependencies
//! beyond `miso-midi-core`. It deliberately does not use Criterion so the raw
//! JSON, loop calibration, and floor assumptions are visible in one binary.

#![allow(clippy::cast_precision_loss)]

use std::{
    env,
    error::Error,
    fmt::Write as _,
    fs,
    hint::black_box,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

use miso_midi_core::{TickScore, parse_score_smf};

const DEFAULT_DATASETS: &[&str] = &["tiny", "normal", "huge", "mahler"];
const DEFAULT_SAMPLES: usize = 30;
const DEFAULT_WARMUP: usize = 5;
const DEFAULT_MIN_SAMPLE_NS: u64 = 50_000_000;
const MAX_CALIBRATION_ITERATIONS: usize = 1 << 30;
const REPORT_SCHEMA: &str = "miso-native-score-benchmark/v1";

const EXPECTED_CORPORA: &[CorpusExpectation] = &[
    CorpusExpectation {
        name: "tiny",
        sha256: "39da22e3a55fdf78b68855e8ed870ccfbf3e5d077401fba7174773f7fa7c92d7",
        semantic_sha256: "bd36b66d133db7772eb2bc5e81e7a1c9ea4a62561de0131a9465ba73c9491acc",
        counts: SemanticCounts {
            tracks: 1,
            notes: 16,
            controls: 3,
            pitch_bends: 1,
            pedals: 1,
            lyrics: 0,
            tempos: 0,
            time_signatures: 0,
            key_signatures: 0,
            markers: 1,
        },
    },
    CorpusExpectation {
        name: "normal",
        sha256: "4b62f8bbd60175f610097817e1759514297f694a46320e1f3d770dbb88c94f97",
        semantic_sha256: "d75cb3bb06a230b8bbbb371e32cf86f5aeaa2a4c1ea098f7f5f371eb559271f1",
        counts: SemanticCounts {
            tracks: 8,
            notes: 16_000,
            controls: 272,
            pitch_bends: 64,
            pedals: 8,
            lyrics: 0,
            tempos: 0,
            time_signatures: 0,
            key_signatures: 0,
            markers: 32,
        },
    },
    CorpusExpectation {
        name: "huge",
        sha256: "90d7ad33e14e80149d8cd2c3d0dae204de9b2ec4670b850593864111245bd40f",
        semantic_sha256: "fe10b416f2f7a65925f38e2a66f201b427040c3243d2b7c818bde3297b12d37c",
        counts: SemanticCounts {
            tracks: 16,
            notes: 192_000,
            controls: 3_040,
            pitch_bends: 752,
            pedals: 16,
            lyrics: 0,
            tempos: 0,
            time_signatures: 0,
            key_signatures: 0,
            markers: 384,
        },
    },
    CorpusExpectation {
        name: "mahler",
        sha256: "35a59329ab8f1f86ec2602bb5293b9fbddc694e512aafa00e310cb8da237f302",
        semantic_sha256: "d8fcfebd208541d7791fc0dab49b561893a7c50180ccbcc61b7049e009013f69",
        counts: SemanticCounts {
            tracks: 51,
            notes: 60_411,
            controls: 36_287,
            pitch_bends: 0,
            pedals: 0,
            lyrics: 0,
            tempos: 177,
            time_signatures: 97,
            key_signatures: 97,
            markers: 97,
        },
    },
];

type BenchResult<T> = Result<T, Box<dyn Error>>;

#[derive(Clone, Debug, Eq, PartialEq)]
struct ScoreSummary {
    tracks: usize,
    notes: usize,
    controls: usize,
    pitch_bends: usize,
    pedals: usize,
    lyrics: usize,
    tempos: usize,
    time_signatures: usize,
    key_signatures: usize,
    markers: usize,
    heap_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SemanticCounts {
    tracks: usize,
    notes: usize,
    controls: usize,
    pitch_bends: usize,
    pedals: usize,
    lyrics: usize,
    tempos: usize,
    time_signatures: usize,
    key_signatures: usize,
    markers: usize,
}

impl SemanticCounts {
    const fn semantic_events(self) -> usize {
        self.notes
            + self.controls
            + self.pitch_bends
            + self.pedals
            + self.lyrics
            + self.tempos
            + self.time_signatures
            + self.key_signatures
            + self.markers
    }

    fn json(self) -> String {
        format!(
            concat!(
                "{{\"tracks\":{},\"notes\":{},\"controls\":{},",
                "\"pitch_bends\":{},\"pedals\":{},\"lyrics\":{},",
                "\"time_signatures\":{},\"key_signatures\":{},",
                "\"tempos\":{},\"markers\":{}}}"
            ),
            self.tracks,
            self.notes,
            self.controls,
            self.pitch_bends,
            self.pedals,
            self.lyrics,
            self.time_signatures,
            self.key_signatures,
            self.tempos,
            self.markers,
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct CorpusExpectation {
    name: &'static str,
    sha256: &'static str,
    semantic_sha256: &'static str,
    counts: SemanticCounts,
}

impl ScoreSummary {
    fn from_score(score: &TickScore) -> Self {
        Self {
            tracks: score.tracks().len(),
            notes: score.note_count(),
            controls: score.controls().len(),
            pitch_bends: score.pitch_bends().len(),
            pedals: score.pedals().len(),
            lyrics: score.lyrics().len(),
            tempos: score.tempos().len(),
            time_signatures: score.time_signatures().len(),
            key_signatures: score.key_signatures().len(),
            markers: score.markers().len(),
            heap_bytes: score.heap_bytes(),
        }
    }

    fn semantic_events(&self) -> usize {
        self.counts().semantic_events()
    }

    const fn counts(&self) -> SemanticCounts {
        SemanticCounts {
            tracks: self.tracks,
            notes: self.notes,
            controls: self.controls,
            pitch_bends: self.pitch_bends,
            pedals: self.pedals,
            lyrics: self.lyrics,
            tempos: self.tempos,
            time_signatures: self.time_signatures,
            key_signatures: self.key_signatures,
            markers: self.markers,
        }
    }

    fn json(&self) -> String {
        format!(
            concat!(
                "{{\"tracks\":{},\"notes\":{},\"semantic_events\":{},",
                "\"controls\":{},\"pitch_bends\":{},\"pedals\":{},",
                "\"lyrics\":{},\"tempos\":{},\"time_signatures\":{},",
                "\"key_signatures\":{},\"markers\":{},\"heap_bytes\":{}}}"
            ),
            self.tracks,
            self.notes,
            self.semantic_events(),
            self.controls,
            self.pitch_bends,
            self.pedals,
            self.lyrics,
            self.tempos,
            self.time_signatures,
            self.key_signatures,
            self.markers,
            self.heap_bytes,
        )
    }
}

const SCORE_CONTRACT_MAGIC: &[u8] = b"MISO-SCORE-CONTRACT\0\x01";

#[allow(clippy::too_many_lines)]
fn score_semantic_sha256(score: &TickScore) -> BenchResult<String> {
    fn signed(output: &mut Vec<u8>, value: i64) {
        output.extend_from_slice(&value.to_be_bytes());
    }
    fn unsigned_count(output: &mut Vec<u8>, value: usize) -> BenchResult<()> {
        output.extend_from_slice(&u64::try_from(value)?.to_be_bytes());
        Ok(())
    }
    fn time(value: u64) -> BenchResult<i64> {
        Ok(i64::try_from(value)?)
    }
    fn text(output: &mut Vec<u8>, value: &[u8]) -> BenchResult<()> {
        unsigned_count(output, value.len())?;
        output.extend_from_slice(value);
        Ok(())
    }

    let mut output = Vec::new();
    output.extend_from_slice(SCORE_CONTRACT_MAGIC);
    signed(&mut output, i64::from(score.ticks_per_quarter()));
    unsigned_count(&mut output, score.tracks().len())?;
    for (index, track) in score.tracks().iter().enumerate() {
        text(
            &mut output,
            score
                .text(track.name())
                .ok_or("track name range is outside score text data")?,
        )?;
        signed(&mut output, i64::from(track.program()));
        output.push(u8::from(track.is_drum()));

        let notes = score
            .track_notes(index)
            .ok_or("track note range is absent")?;
        unsigned_count(&mut output, notes.len())?;
        for event in notes {
            signed(&mut output, time(event.time())?);
            signed(&mut output, time(event.duration())?);
            signed(&mut output, i64::from(event.pitch()));
            signed(&mut output, i64::from(event.velocity()));
        }
        let controls = score
            .track_controls(index)
            .ok_or("track control range is absent")?;
        unsigned_count(&mut output, controls.len())?;
        for event in controls {
            signed(&mut output, time(event.time())?);
            signed(&mut output, i64::from(event.number()));
            signed(&mut output, i64::from(event.value()));
        }
        let bends = score
            .track_pitch_bends(index)
            .ok_or("track pitch-bend range is absent")?;
        unsigned_count(&mut output, bends.len())?;
        for event in bends {
            signed(&mut output, time(event.time())?);
            signed(&mut output, i64::from(event.value()));
        }
        let pedals = score
            .track_pedals(index)
            .ok_or("track pedal range is absent")?;
        unsigned_count(&mut output, pedals.len())?;
        for event in pedals {
            signed(&mut output, time(event.time())?);
            signed(&mut output, time(event.duration())?);
        }
        let lyrics = score
            .track_lyrics(index)
            .ok_or("track lyric range is absent")?;
        unsigned_count(&mut output, lyrics.len())?;
        for event in lyrics {
            signed(&mut output, time(event.time())?);
            text(
                &mut output,
                score
                    .text(event.text())
                    .ok_or("lyric range is outside score text data")?,
            )?;
        }
    }
    unsigned_count(&mut output, score.time_signatures().len())?;
    for event in score.time_signatures() {
        signed(&mut output, time(event.time())?);
        signed(&mut output, i64::from(event.numerator()));
        signed(&mut output, i64::try_from(event.denominator())?);
    }
    unsigned_count(&mut output, score.key_signatures().len())?;
    for event in score.key_signatures() {
        signed(&mut output, time(event.time())?);
        signed(&mut output, i64::from(event.key()));
        signed(&mut output, i64::from(event.tonality()));
    }
    unsigned_count(&mut output, score.tempos().len())?;
    for event in score.tempos() {
        signed(&mut output, time(event.time())?);
        signed(&mut output, i64::from(event.microseconds_per_quarter()));
    }
    unsigned_count(&mut output, score.markers().len())?;
    for event in score.markers() {
        signed(&mut output, time(event.time())?);
        text(
            &mut output,
            score
                .text(event.text())
                .ok_or("marker range is outside score text data")?,
        )?;
    }
    Ok(sha256_hex(&output))
}

#[derive(Clone, Debug)]
struct Distribution {
    iterations: usize,
    samples_ns_per_operation: Vec<f64>,
}

impl Distribution {
    fn sorted_samples(&self) -> Vec<f64> {
        let mut values = self.samples_ns_per_operation.clone();
        values.sort_by(f64::total_cmp);
        values
    }

    fn median_ns(&self) -> f64 {
        let values = self.sorted_samples();
        let middle = values.len() / 2;
        if values.len().is_multiple_of(2) {
            f64::midpoint(values[middle - 1], values[middle])
        } else {
            values[middle]
        }
    }

    fn mean_ns(&self) -> f64 {
        self.samples_ns_per_operation.iter().sum::<f64>()
            / self.samples_ns_per_operation.len() as f64
    }

    fn min_ns(&self) -> f64 {
        self.samples_ns_per_operation
            .iter()
            .copied()
            .reduce(f64::min)
            .expect("distribution has samples")
    }

    fn max_ns(&self) -> f64 {
        self.samples_ns_per_operation
            .iter()
            .copied()
            .reduce(f64::max)
            .expect("distribution has samples")
    }

    fn json(&self) -> String {
        format!(
            concat!(
                "{{\"iterations\":{},\"samples_ns_per_operation\":{},",
                "\"median_ns\":{:.6},\"mean_ns\":{:.6},",
                "\"min_ns\":{:.6},\"max_ns\":{:.6}}}"
            ),
            self.iterations,
            json_f64_list(&self.samples_ns_per_operation),
            self.median_ns(),
            self.mean_ns(),
            self.min_ns(),
            self.max_ns(),
        )
    }
}

#[derive(Clone, Debug)]
struct FloorResult {
    name: &'static str,
    assumption: &'static str,
    work_bytes: usize,
    distribution: Distribution,
}

impl FloorResult {
    fn json(&self, parse_median_ns: f64) -> String {
        let ratio = diagnostic_floor_ratio(parse_median_ns, self.distribution.median_ns())
            .map_or_else(|| "null".to_owned(), |value| format!("{value:.6}"));
        format!(
            concat!(
                "{{\"name\":\"{}\",\"assumption\":\"{}\",",
                "\"work_bytes\":{},\"median_ns_per_byte\":{:.9},",
                "\"parse_median_to_floor_median_ratio\":{},",
                "\"distribution\":{}}}"
            ),
            self.name,
            self.assumption,
            self.work_bytes,
            self.distribution.median_ns() / self.work_bytes as f64,
            ratio,
            self.distribution.json(),
        )
    }
}

#[derive(Clone, Debug)]
struct Options {
    corpus_dir: PathBuf,
    datasets: Vec<String>,
    samples: usize,
    warmup: usize,
    iterations: Option<usize>,
    min_sample: Duration,
    include_floors: bool,
    verify_only: bool,
    output: Option<PathBuf>,
}

fn main() -> BenchResult<()> {
    let options = parse_options()?;
    if options.verify_only {
        verify_all(&options)?;
        println!("native Miso fixed-contract verification: ok");
        return Ok(());
    }
    let report = build_report(&options)?;
    if let Some(path) = options.output {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, report)?;
    } else {
        println!("{report}");
    }
    Ok(())
}

fn parse_options() -> BenchResult<Options> {
    let mut corpus_dir = PathBuf::from("benchmarks/corpus");
    let mut datasets: Vec<String> = DEFAULT_DATASETS.iter().map(ToString::to_string).collect();
    let mut samples = DEFAULT_SAMPLES;
    let mut warmup = DEFAULT_WARMUP;
    let mut iterations = None;
    let mut min_sample = Duration::from_nanos(DEFAULT_MIN_SAMPLE_NS);
    let mut include_floors = true;
    let mut verify_only = false;
    let mut output = None;
    let mut args = env::args().skip(1);

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--corpus-dir" => corpus_dir = PathBuf::from(next_argument(&mut args, "--corpus-dir")?),
            "--datasets" => {
                let values = next_argument(&mut args, "--datasets")?;
                datasets = values.split(',').map(ToString::to_string).collect();
            }
            "--samples" => samples = next_argument(&mut args, "--samples")?.parse()?,
            "--warmup" => warmup = next_argument(&mut args, "--warmup")?.parse()?,
            "--iterations" => {
                let value: usize = next_argument(&mut args, "--iterations")?.parse()?;
                iterations = (value != 0).then_some(value);
            }
            "--min-sample-ns" => {
                min_sample =
                    Duration::from_nanos(next_argument(&mut args, "--min-sample-ns")?.parse()?);
            }
            "--parse-only" | "--no-floors" => include_floors = false,
            "--verify-only" => verify_only = true,
            "--output" => output = Some(PathBuf::from(next_argument(&mut args, "--output")?)),
            "--help" | "-h" => {
                println!(
                    "Usage: cargo run -p miso-midi-native-score-bench --release -- \\\n                     [--corpus-dir PATH] [--datasets tiny,normal,huge,mahler] \\\n                     [--samples N] [--warmup N] [--iterations N|0] \\\n                     [--min-sample-ns N] [--output PATH]"
                );
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {argument}").into()),
        }
    }

    if datasets.is_empty() || datasets.iter().any(String::is_empty) {
        return Err("--datasets must contain at least one non-empty name".into());
    }
    if samples == 0 {
        return Err("--samples must be positive".into());
    }
    if min_sample.is_zero() {
        return Err("--min-sample-ns must be positive".into());
    }
    Ok(Options {
        corpus_dir,
        datasets,
        samples,
        warmup,
        iterations,
        min_sample,
        include_floors,
        verify_only,
        output,
    })
}

fn next_argument(args: &mut impl Iterator<Item = String>, name: &str) -> BenchResult<String> {
    args.next()
        .ok_or_else(|| format!("{name} requires a value").into())
}

fn build_report(options: &Options) -> BenchResult<String> {
    let mut datasets = Vec::with_capacity(options.datasets.len());
    for dataset in &options.datasets {
        datasets.push(measure_dataset(dataset, options)?);
    }

    Ok(format!(
        concat!(
            "{{\"schema\":\"{}\",\"benchmark\":\"parse_score_smf\",",
            "\"method\":\"warm in-memory single-thread distributions; parse and score destruction timed together; fixed corpus hash/full semantic digest/cardinalities verified outside timing\",",
            "\"floor_disclaimer\":\"Floors and parse/floor ratios are diagnostic assumptions, not theoretical limits, percent-of-theoretical values, or product claims.\",",
            "\"machine\":{},\"configuration\":{},\"datasets\":[{}]}}"
        ),
        REPORT_SCHEMA,
        machine_metadata_json(),
        configuration_json(options),
        datasets.join(","),
    ))
}

fn verify_all(options: &Options) -> BenchResult<()> {
    for name in &options.datasets {
        let data = fs::read(options.corpus_dir.join(format!("{name}.mid")))?;
        let expectation = corpus_expectation(name)?;
        let score = parse_score_smf(&data)?;
        verify_reference(name, &data, &score, expectation)?;
    }
    Ok(())
}

fn measure_dataset(name: &str, options: &Options) -> BenchResult<String> {
    let path = options.corpus_dir.join(format!("{name}.mid"));
    let data = fs::read(&path)?;
    let expectation = corpus_expectation(name)?;
    let reference = parse_score_smf(&data)?;
    let expected = ScoreSummary::from_score(&reference);
    verify_reference(name, &data, &reference, expectation)?;

    let mut parse_operation = || {
        let score =
            parse_score_smf(black_box(data.as_slice())).expect("preflighted SMF must parse");
        black_box(score);
    };
    let parse_distribution = measure_distribution(
        &mut parse_operation,
        options.samples,
        options.warmup,
        options.iterations,
        options.min_sample,
    );
    let post_timing = parse_score_smf(&data)?;
    verify_reference(name, &data, &post_timing, expectation)?;

    let floors = if options.include_floors {
        measure_floors(&data, expected.heap_bytes, options)
    } else {
        Vec::new()
    };
    let parse_median_ns = parse_distribution.median_ns();
    Ok(format!(
        concat!(
            "{{\"dataset\":\"{}\",\"input_path\":\"{}\",",
            "\"input_bytes\":{},\"input_sha256\":\"{}\",",
            "\"semantic_contract\":{{\"schema\":\"miso-score-contract/v1\",\"sha256\":\"{}\",\"summary\":{}}},",
            "\"semantic_cardinalities\":{},\"parse_score_smf\":{},",
            "\"parse_median_ns_per_byte\":{:.9},",
            "\"parse_median_ns_per_semantic_event\":{:.9},",
            "\"parse_median_ns_per_note\":{:.9},\"diagnostic_floors\":[{}]}}"
        ),
        json_escape(name),
        json_escape(&path.display().to_string()),
        data.len(),
        sha256_hex(&data),
        expectation.semantic_sha256,
        expectation.counts.json(),
        expected.json(),
        parse_distribution.json(),
        parse_median_ns / data.len() as f64,
        per_count(parse_median_ns, expected.semantic_events()),
        per_count(parse_median_ns, expected.notes),
        floors
            .iter()
            .map(|floor| floor.json(parse_median_ns))
            .collect::<Vec<_>>()
            .join(","),
    ))
}

fn corpus_expectation(name: &str) -> BenchResult<&'static CorpusExpectation> {
    EXPECTED_CORPORA
        .iter()
        .find(|expectation| expectation.name == name)
        .ok_or_else(|| {
            format!(
                "no authoritative native benchmark expectation for {name:?}; use one of {}",
                DEFAULT_DATASETS.join(",")
            )
            .into()
        })
}

fn verify_reference(
    name: &str,
    data: &[u8],
    observed: &TickScore,
    expectation: &CorpusExpectation,
) -> BenchResult<()> {
    let hash = sha256_hex(data);
    if hash != expectation.sha256 {
        return Err(format!(
            "corpus hash mismatch for {name}: expected {}, observed {hash}",
            expectation.sha256
        )
        .into());
    }
    let summary = ScoreSummary::from_score(observed);
    if summary.counts() != expectation.counts {
        return Err(format!(
            "semantic cardinality mismatch for {name}: expected {:?}, observed {:?}",
            expectation.counts,
            summary.counts()
        )
        .into());
    }
    let semantic_sha256 = score_semantic_sha256(observed)?;
    if semantic_sha256 != expectation.semantic_sha256 {
        return Err(format!(
            "semantic digest mismatch for {name}: expected {}, observed {semantic_sha256}",
            expectation.semantic_sha256
        )
        .into());
    }
    Ok(())
}

fn measure_floors(data: &[u8], output_bytes: usize, options: &Options) -> Vec<FloorResult> {
    let mut byte_touch = || {
        let input = black_box(data);
        let checksum = input
            .iter()
            .fold(0_u64, |sum, byte| sum.wrapping_add(u64::from(*byte)));
        black_box(checksum);
    };
    let mut allocation = || {
        let output = Vec::<u8>::with_capacity(black_box(output_bytes));
        black_box(output);
    };
    let mut output_write = || {
        let mut output = Vec::with_capacity(black_box(output_bytes));
        output.resize(output_bytes, 0);
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = u8::try_from(index & 0xff).expect("masked index fits u8");
        }
        black_box(output);
    };
    let settings = (
        options.samples,
        options.warmup,
        options.iterations,
        options.min_sample,
    );
    vec![
        FloorResult {
            name: "input_byte_touch",
            assumption: "reads every input byte once and performs a dependency-preserving checksum",
            work_bytes: data.len(),
            distribution: measure_distribution(
                &mut byte_touch,
                settings.0,
                settings.1,
                settings.2,
                settings.3,
            ),
        },
        FloorResult {
            name: "output_allocation_request",
            assumption: "requests one contiguous allocation equal to TickScore heap capacity; pages need not be touched",
            work_bytes: output_bytes,
            distribution: measure_distribution(
                &mut allocation,
                settings.0,
                settings.1,
                settings.2,
                settings.3,
            ),
        },
        FloorResult {
            name: "output_column_allocate_and_write",
            assumption: "allocates one contiguous byte column equal to TickScore heap capacity and writes every byte once",
            work_bytes: output_bytes,
            distribution: measure_distribution(
                &mut output_write,
                settings.0,
                settings.1,
                settings.2,
                settings.3,
            ),
        },
    ]
}

fn measure_distribution(
    operation: &mut impl FnMut(),
    samples: usize,
    warmup: usize,
    requested_iterations: Option<usize>,
    min_sample: Duration,
) -> Distribution {
    for _ in 0..warmup {
        operation();
    }
    let iterations =
        requested_iterations.unwrap_or_else(|| calibrate_iterations(operation, min_sample));
    let mut values = Vec::with_capacity(samples);
    for _ in 0..samples {
        let start = Instant::now();
        for _ in 0..iterations {
            operation();
        }
        values.push(start.elapsed().as_nanos() as f64 / iterations as f64);
    }
    Distribution {
        iterations,
        samples_ns_per_operation: values,
    }
}

fn calibrate_iterations(operation: &mut impl FnMut(), min_sample: Duration) -> usize {
    let mut iterations = 1;
    loop {
        let start = Instant::now();
        for _ in 0..iterations {
            operation();
        }
        if start.elapsed() >= min_sample || iterations >= MAX_CALIBRATION_ITERATIONS {
            return iterations;
        }
        iterations = next_calibration_iterations(iterations);
    }
}

fn next_calibration_iterations(iterations: usize) -> usize {
    iterations.saturating_mul(2).min(MAX_CALIBRATION_ITERATIONS)
}

fn per_count(value: f64, count: usize) -> f64 {
    if count == 0 {
        0.0
    } else {
        value / count as f64
    }
}

fn diagnostic_floor_ratio(parse_median_ns: f64, floor_median_ns: f64) -> Option<f64> {
    (floor_median_ns > 0.0).then(|| parse_median_ns / floor_median_ns)
}

fn configuration_json(options: &Options) -> String {
    format!(
        concat!(
            "{{\"samples\":{},\"warmup\":{},\"iterations\":{},",
            "\"min_sample_ns\":{},\"datasets\":{},\"parse_only\":{},",
            "\"timed_operation\":\"parse_score_and_destroy\"}}"
        ),
        options.samples,
        options.warmup,
        options
            .iterations
            .map_or_else(|| "\"auto\"".to_owned(), |value| value.to_string()),
        options.min_sample.as_nanos(),
        json_string_list(&options.datasets),
        !options.include_floors,
    )
}

fn machine_metadata_json() -> String {
    format!(
        concat!(
            "{{\"target_arch\":\"{}\",\"target_os\":\"{}\",",
            "\"rustc\":\"{}\",\"cargo_profile\":\"{}\",",
            "\"debug_assertions\":{},\"cpu_affinity\":\"{}\",",
            "\"cpu_model\":\"{}\",\"cpu_governor\":\"{}\",",
            "\"kernel_release\":\"{}\",\"rust_release_profile_config\":",
            "{{\"source\":\"workspace [profile.release]\",\"lto\":\"thin\",\"codegen_units\":1,\"panic\":\"abort\"}}}}"
        ),
        env::consts::ARCH,
        env::consts::OS,
        json_escape(&rustc_version()),
        env!("MISO_NATIVE_SCORE_CARGO_PROFILE"),
        cfg!(debug_assertions),
        json_escape(
            &linux_status_value("Cpus_allowed_list").unwrap_or_else(|| "unknown".to_owned())
        ),
        json_escape(&linux_cpu_model().unwrap_or_else(|| "unknown".to_owned())),
        json_escape(&linux_cpu_governor().unwrap_or_else(|| "unknown".to_owned())),
        json_escape(&linux_kernel_release().unwrap_or_else(|| "unknown".to_owned())),
    )
}

fn rustc_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map_or_else(|| "unavailable".to_owned(), |value| value.trim().to_owned())
}

fn linux_status_value(key: &str) -> Option<String> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    proc_key_value(&status, key)
}

fn linux_cpu_model() -> Option<String> {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").ok()?;
    proc_key_value(&cpuinfo, "model name")
}

fn first_cpu_from_affinity(value: &str) -> Option<u32> {
    value.trim_start().split(['-', ',']).next()?.parse().ok()
}

fn linux_cpu_governor() -> Option<String> {
    (env::consts::OS == "linux").then_some(())?;
    let affinity = linux_status_value("Cpus_allowed_list")?;
    let cpu = first_cpu_from_affinity(&affinity)?;
    fs::read_to_string(format!(
        "/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor"
    ))
    .ok()
    .map(|value| value.trim().to_owned())
    .filter(|value| !value.is_empty())
}

fn linux_kernel_release() -> Option<String> {
    (env::consts::OS == "linux").then_some(())?;
    Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn proc_key_value(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        (name.trim() == key).then(|| value.trim().to_owned())
    })
}

fn json_string_list(values: &[String]) -> String {
    let body = values
        .iter()
        .map(|value| format!("\"{}\"", json_escape(value)))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{body}]")
}

fn json_f64_list(values: &[f64]) -> String {
    let body = values
        .iter()
        .map(|value| format!("{value:.6}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{body}]")
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                write!(escaped, "\\u{:04x}", u32::from(character))
                    .expect("writing to String cannot fail");
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = sha256(data);
    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

#[allow(clippy::many_single_char_names, clippy::too_many_lines)]
fn sha256(data: &[u8]) -> [u8; 32] {
    const INITIAL: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];
    let mut state = INITIAL;
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(data.len() + 72);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() + 8) % 64 != 0 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut words = [0_u32; 64];
        for (index, bytes) in chunk.chunks_exact(4).enumerate() {
            words[index] = u32::from_be_bytes(bytes.try_into().expect("four-byte chunk"));
        }
        for index in 16..64 {
            let sigma0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let sigma1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(sigma0)
                .wrapping_add(words[index - 7])
                .wrapping_add(sigma1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ ((!e) & g);
            let temporary1 = h
                .wrapping_add(sum1)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temporary2 = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temporary1);
            d = c;
            c = b;
            b = a;
            a = temporary1.wrapping_add(temporary2);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
    let mut output = [0_u8; 32];
    for (index, word) in state.iter().enumerate() {
        output[index * 4..(index + 1) * 4].copy_from_slice(&word.to_be_bytes());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_standard_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn distribution_statistics_are_independent_of_input_order() {
        let distribution = Distribution {
            iterations: 4,
            samples_ns_per_operation: vec![10.0, 2.0, 6.0, 4.0],
        };
        assert!((distribution.median_ns() - 5.0).abs() < f64::EPSILON);
        assert!((distribution.mean_ns() - 5.5).abs() < f64::EPSILON);
        assert!((distribution.min_ns() - 2.0).abs() < f64::EPSILON);
        assert!((distribution.max_ns() - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn calibration_growth_is_bounded_and_deterministic() {
        assert_eq!(next_calibration_iterations(1), 2);
        assert_eq!(
            next_calibration_iterations(MAX_CALIBRATION_ITERATIONS),
            MAX_CALIBRATION_ITERATIONS
        );
    }

    #[test]
    fn json_escapes_control_characters() {
        assert_eq!(json_escape("a\n\"\\"), "a\\n\\\"\\\\");
    }

    #[test]
    fn configuration_metadata_preserves_explicit_iteration_mode() {
        let options = Options {
            corpus_dir: PathBuf::from("unused"),
            datasets: vec!["tiny".to_owned(), "normal".to_owned()],
            samples: 7,
            warmup: 2,
            iterations: Some(11),
            min_sample: Duration::from_nanos(123),
            include_floors: false,
            verify_only: false,
            output: None,
        };
        let metadata = configuration_json(&options);
        assert!(metadata.contains("\"samples\":7"));
        assert!(metadata.contains("\"iterations\":11"));
        assert!(metadata.contains("\"datasets\":[\"tiny\",\"normal\"]"));
        assert!(metadata.contains("\"parse_only\":true"));
    }

    #[test]
    fn proc_key_parser_trims_linux_cpuinfo_keys() {
        let cpuinfo = "processor\t: 0\nmodel name\t: Test CPU\n";
        assert_eq!(
            proc_key_value(cpuinfo, "model name"),
            Some("Test CPU".to_owned())
        );
    }

    #[test]
    fn affinity_cpu_parser_uses_the_first_requested_cpu() {
        assert_eq!(first_cpu_from_affinity("4-7,12"), Some(4));
        assert_eq!(first_cpu_from_affinity("  12,14"), Some(12));
        assert_eq!(first_cpu_from_affinity("unknown"), None);
    }

    #[test]
    fn diagnostic_ratio_is_not_a_percentage() {
        let ratio = diagnostic_floor_ratio(20.0, 5.0).expect("nonzero floor has a ratio");
        assert!((ratio - 4.0).abs() < f64::EPSILON);
        assert_eq!(diagnostic_floor_ratio(20.0, 0.0), None);
    }
}
