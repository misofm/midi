use miso_midi_core::{
    OwnedSmf as CoreOwnedSmf, ScanSummary as CoreScanSummary,
    ScoreParseLimits as CoreScoreParseLimits, ScoreParseMode, ScoreParseOptions,
    TickScore as CoreTickScore, parse_score_smf, parse_score_smf_with_options, parse_smf, scan_smf,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};

#[pyclass(frozen, module = "miso_midi._native", skip_from_py_object)]
#[derive(Clone)]
struct ScanSummary {
    #[pyo3(get)]
    format: u16,
    #[pyo3(get)]
    tracks: u16,
    #[pyo3(get)]
    division: u16,
    #[pyo3(get)]
    events: u64,
    #[pyo3(get)]
    channel_events: u64,
    #[pyo3(get)]
    meta_events: u64,
    #[pyo3(get)]
    sysex_events: u64,
    #[pyo3(get)]
    payload_bytes: u64,
    #[pyo3(get)]
    max_delta_ticks: u32,
    #[pyo3(get)]
    bytes_consumed: usize,
    #[pyo3(get)]
    trailing_bytes: usize,
}

impl From<CoreScanSummary> for ScanSummary {
    fn from(summary: CoreScanSummary) -> Self {
        Self {
            format: summary.header.format as u16,
            tracks: summary.header.track_count,
            division: summary.header.division,
            events: summary.events,
            channel_events: summary.channel_events,
            meta_events: summary.meta_events,
            sysex_events: summary.sysex_events,
            payload_bytes: summary.payload_bytes,
            max_delta_ticks: summary.max_delta_ticks,
            bytes_consumed: summary.bytes_consumed,
            trailing_bytes: summary.trailing_bytes,
        }
    }
}

/// A compact Rust-owned MIDI file.
#[pyclass(frozen, module = "miso_midi._native", skip_from_py_object)]
struct MidiFile {
    inner: CoreOwnedSmf,
}

#[pymethods]
impl MidiFile {
    #[new]
    fn new(data: &[u8]) -> PyResult<Self> {
        parse_owned(data)
    }

    #[getter]
    fn format(&self) -> u16 {
        self.inner.header().format as u16
    }

    #[getter]
    fn track_count(&self) -> u16 {
        self.inner.header().track_count
    }

    #[getter]
    fn division(&self) -> u16 {
        self.inner.header().division
    }

    #[getter]
    fn event_count(&self) -> usize {
        self.inner.events().len()
    }

    #[getter]
    fn heap_bytes(&self) -> usize {
        self.inner.heap_bytes()
    }

    #[getter]
    fn bytes_consumed(&self) -> usize {
        self.inner.bytes_consumed()
    }

    #[getter]
    fn trailing_bytes(&self) -> usize {
        self.inner.trailing_bytes()
    }

    #[getter]
    fn track_lengths(&self) -> Vec<u32> {
        self.inner
            .tracks()
            .iter()
            .map(|track| track.len())
            .collect()
    }

    /// Materialize normalized records as
    /// `(track, delta_ticks, status, meta_type, data)` tuples.
    ///
    /// `SysEx` framing is normalized to status `0xF0`, with optional leading
    /// `0xF0` and trailing `0xF7` bytes removed from the payload.
    fn semantic_records(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let records = PyList::empty(py);
        for (track_index, track) in self.inner.tracks().iter().enumerate() {
            let start = usize::try_from(track.start())
                .map_err(|_| PyValueError::new_err("track start does not fit usize"))?;
            let events = self
                .inner
                .track_events(track_index)
                .ok_or_else(|| PyValueError::new_err("invalid internal track range"))?;
            for (local_index, event) in events.iter().copied().enumerate() {
                let global_index = start + local_index;
                let raw_data = self
                    .inner
                    .event_data(global_index)
                    .ok_or_else(|| PyValueError::new_err("invalid internal payload range"))?;
                let (status, data) = if matches!(event.status(), 0xf0 | 0xf7) {
                    (0xf0, normalize_sysex(raw_data))
                } else {
                    (event.status(), raw_data)
                };
                records.append((
                    track_index,
                    event.delta_ticks(),
                    status,
                    event.meta_type(),
                    PyBytes::new(py, data),
                ))?;
            }
        }
        Ok(records.unbind())
    }

    fn __len__(&self) -> usize {
        self.inner.events().len()
    }

    fn __repr__(&self) -> String {
        format!(
            "MidiFile(format={}, tracks={}, division={}, events={})",
            self.inner.header().format as u16,
            self.inner.header().track_count,
            self.inner.header().division,
            self.inner.events().len()
        )
    }
}

fn normalize_sysex(mut data: &[u8]) -> &[u8] {
    if data.first() == Some(&0xf0) {
        data = &data[1..];
    }
    if data.last() == Some(&0xf7) {
        data = &data[..data.len() - 1];
    }
    data
}

fn parse_owned(data: &[u8]) -> PyResult<MidiFile> {
    parse_smf(data)
        .map(|inner| MidiFile { inner })
        .map_err(|error| PyValueError::new_err(error.to_string()))
}

/// Finite resource ceilings for the default Python score parser.
///
/// Construct this with keyword arguments to lower or raise the documented
/// finite ceilings. `parse_score(..., limits=None)` uses these core defaults;
/// it never means unlimited parsing.
#[pyclass(frozen, module = "miso_midi._native", skip_from_py_object)]
#[derive(Clone)]
struct ScoreParseLimits {
    inner: CoreScoreParseLimits,
}

#[pymethods]
impl ScoreParseLimits {
    #[new]
    #[pyo3(signature = (*, max_input_bytes = CoreScoreParseLimits::DEFAULT.max_input_bytes, max_source_tracks = CoreScoreParseLimits::DEFAULT.max_source_tracks, max_track_bytes = CoreScoreParseLimits::DEFAULT.max_track_bytes, max_events = CoreScoreParseLimits::DEFAULT.max_events, max_note_starts = CoreScoreParseLimits::DEFAULT.max_note_starts, max_text_bytes = CoreScoreParseLimits::DEFAULT.max_text_bytes))]
    fn new(
        max_input_bytes: usize,
        max_source_tracks: u16,
        max_track_bytes: usize,
        max_events: usize,
        max_note_starts: usize,
        max_text_bytes: usize,
    ) -> Self {
        Self {
            inner: CoreScoreParseLimits {
                max_input_bytes,
                max_source_tracks,
                max_track_bytes,
                max_events,
                max_note_starts,
                max_text_bytes,
            },
        }
    }

    #[getter]
    fn max_input_bytes(&self) -> usize {
        self.inner.max_input_bytes
    }

    #[getter]
    fn max_source_tracks(&self) -> u16 {
        self.inner.max_source_tracks
    }

    #[getter]
    fn max_track_bytes(&self) -> usize {
        self.inner.max_track_bytes
    }

    #[getter]
    fn max_events(&self) -> usize {
        self.inner.max_events
    }

    #[getter]
    fn max_note_starts(&self) -> usize {
        self.inner.max_note_starts
    }

    #[getter]
    fn max_text_bytes(&self) -> usize {
        self.inner.max_text_bytes
    }

    fn __repr__(&self) -> String {
        format!(
            "ScoreParseLimits(max_input_bytes={}, max_source_tracks={}, max_track_bytes={}, max_events={}, max_note_starts={}, max_text_bytes={})",
            self.inner.max_input_bytes,
            self.inner.max_source_tracks,
            self.inner.max_track_bytes,
            self.inner.max_events,
            self.inner.max_note_starts,
            self.inner.max_text_bytes,
        )
    }
}

/// A compact native tick score. Per-event Python objects are created only by
/// [`Score::semantic_records`], never by parsing.
#[pyclass(frozen, module = "miso_midi._native", skip_from_py_object)]
struct Score {
    inner: CoreTickScore,
}

#[pymethods]
impl Score {
    #[new]
    #[pyo3(signature = (data, *, limits = None, mode = "compatible"))]
    #[allow(clippy::needless_pass_by_value)] // PyO3 extracts an owned PyRef.
    fn new(data: &[u8], limits: Option<PyRef<'_, ScoreParseLimits>>, mode: &str) -> PyResult<Self> {
        parse_score_owned(data, limits.as_deref(), mode)
    }

    #[getter]
    fn ticks_per_quarter(&self) -> u16 {
        self.inner.ticks_per_quarter()
    }

    #[getter]
    fn track_count(&self) -> usize {
        self.inner.tracks().len()
    }

    #[getter]
    fn note_count(&self) -> usize {
        self.inner.note_count()
    }

    #[getter]
    fn control_count(&self) -> usize {
        self.inner.controls().len()
    }

    #[getter]
    fn pitch_bend_count(&self) -> usize {
        self.inner.pitch_bends().len()
    }

    #[getter]
    fn pedal_count(&self) -> usize {
        self.inner.pedals().len()
    }

    #[getter]
    fn lyric_count(&self) -> usize {
        self.inner.lyrics().len()
    }

    #[getter]
    fn time_signature_count(&self) -> usize {
        self.inner.time_signatures().len()
    }

    #[getter]
    fn key_signature_count(&self) -> usize {
        self.inner.key_signatures().len()
    }

    #[getter]
    fn tempo_count(&self) -> usize {
        self.inner.tempos().len()
    }

    #[getter]
    fn marker_count(&self) -> usize {
        self.inner.markers().len()
    }

    #[getter]
    fn heap_bytes(&self) -> usize {
        self.inner.heap_bytes()
    }

    #[getter]
    fn bytes_consumed(&self) -> usize {
        self.inner.bytes_consumed()
    }

    #[getter]
    fn trailing_bytes(&self) -> usize {
        self.inner.trailing_bytes()
    }

    /// Materialize the benchmark contract's built-in mapping schema in one
    /// native call. This intentionally sits outside parse timing.
    fn semantic_records(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let score = PyDict::new(py);
        score.set_item("tpq", self.inner.ticks_per_quarter())?;

        let tracks = PyList::empty(py);
        for (track_index, track) in self.inner.tracks().iter().copied().enumerate() {
            let record = PyDict::new(py);
            record.set_item("name", self.text(track.name())?)?;
            record.set_item("program", track.program())?;
            record.set_item("is_drum", track.is_drum())?;
            record.set_item("notes", self.track_notes(py, track_index)?)?;
            record.set_item("controls", self.track_controls(py, track_index)?)?;
            record.set_item("pitch_bends", self.track_pitch_bends(py, track_index)?)?;
            record.set_item("pedals", self.track_pedals(py, track_index)?)?;
            record.set_item("lyrics", self.track_lyrics(py, track_index)?)?;
            tracks.append(record)?;
        }
        score.set_item("tracks", tracks)?;
        score.set_item("time_signatures", self.time_signatures(py)?)?;
        score.set_item("key_signatures", self.key_signatures(py)?)?;
        score.set_item("tempos", self.tempos(py)?)?;
        score.set_item("markers", self.markers(py)?)?;
        Ok(score.unbind())
    }

    fn __len__(&self) -> usize {
        self.inner.tracks().len()
    }

    fn __repr__(&self) -> String {
        format!(
            "Score(tpq={}, tracks={}, notes={})",
            self.inner.ticks_per_quarter(),
            self.inner.tracks().len(),
            self.inner.note_count()
        )
    }
}

impl Score {
    fn text(&self, range: miso_midi_core::TextRange) -> PyResult<String> {
        let bytes = self
            .inner
            .text(range)
            .ok_or_else(|| PyValueError::new_err("invalid internal text range"))?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }

    fn track_notes<'py>(&self, py: Python<'py>, track: usize) -> PyResult<Bound<'py, PyList>> {
        let records = PyList::empty(py);
        let events = self
            .inner
            .track_notes(track)
            .ok_or_else(|| PyValueError::new_err("invalid internal note range"))?;
        for event in events {
            let record = PyDict::new(py);
            record.set_item("time", event.time())?;
            record.set_item("duration", event.duration())?;
            record.set_item("pitch", event.pitch())?;
            record.set_item("velocity", event.velocity())?;
            records.append(record)?;
        }
        Ok(records)
    }

    fn track_controls<'py>(&self, py: Python<'py>, track: usize) -> PyResult<Bound<'py, PyList>> {
        let records = PyList::empty(py);
        let events = self
            .inner
            .track_controls(track)
            .ok_or_else(|| PyValueError::new_err("invalid internal control range"))?;
        for event in events {
            let record = PyDict::new(py);
            record.set_item("time", event.time())?;
            record.set_item("number", event.number())?;
            record.set_item("value", event.value())?;
            records.append(record)?;
        }
        Ok(records)
    }

    fn track_pitch_bends<'py>(
        &self,
        py: Python<'py>,
        track: usize,
    ) -> PyResult<Bound<'py, PyList>> {
        let records = PyList::empty(py);
        let events = self
            .inner
            .track_pitch_bends(track)
            .ok_or_else(|| PyValueError::new_err("invalid internal pitch-bend range"))?;
        for event in events {
            let record = PyDict::new(py);
            record.set_item("time", event.time())?;
            record.set_item("value", event.value())?;
            records.append(record)?;
        }
        Ok(records)
    }

    fn track_pedals<'py>(&self, py: Python<'py>, track: usize) -> PyResult<Bound<'py, PyList>> {
        let records = PyList::empty(py);
        let events = self
            .inner
            .track_pedals(track)
            .ok_or_else(|| PyValueError::new_err("invalid internal pedal range"))?;
        for event in events {
            let record = PyDict::new(py);
            record.set_item("time", event.time())?;
            record.set_item("duration", event.duration())?;
            records.append(record)?;
        }
        Ok(records)
    }

    fn track_lyrics<'py>(&self, py: Python<'py>, track: usize) -> PyResult<Bound<'py, PyList>> {
        let records = PyList::empty(py);
        let events = self
            .inner
            .track_lyrics(track)
            .ok_or_else(|| PyValueError::new_err("invalid internal lyric range"))?;
        for event in events {
            let record = PyDict::new(py);
            record.set_item("time", event.time())?;
            record.set_item("text", self.text(event.text())?)?;
            records.append(record)?;
        }
        Ok(records)
    }

    fn time_signatures<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let records = PyList::empty(py);
        for event in self.inner.time_signatures() {
            let record = PyDict::new(py);
            record.set_item("time", event.time())?;
            record.set_item("numerator", event.numerator())?;
            record.set_item("denominator", event.denominator())?;
            records.append(record)?;
        }
        Ok(records)
    }

    fn key_signatures<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let records = PyList::empty(py);
        for event in self.inner.key_signatures() {
            let record = PyDict::new(py);
            record.set_item("time", event.time())?;
            record.set_item("key", event.key())?;
            record.set_item("tonality", event.tonality())?;
            records.append(record)?;
        }
        Ok(records)
    }

    fn tempos<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let records = PyList::empty(py);
        for event in self.inner.tempos() {
            let record = PyDict::new(py);
            record.set_item("time", event.time())?;
            record.set_item("mspq", event.microseconds_per_quarter())?;
            records.append(record)?;
        }
        Ok(records)
    }

    fn markers<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let records = PyList::empty(py);
        for event in self.inner.markers() {
            let record = PyDict::new(py);
            record.set_item("time", event.time())?;
            record.set_item("text", self.text(event.text())?)?;
            records.append(record)?;
        }
        Ok(records)
    }
}

fn parse_score_mode(mode: &str) -> PyResult<ScoreParseMode> {
    match mode {
        "compatible" => Ok(ScoreParseMode::Compatible),
        "strict" => Ok(ScoreParseMode::Strict),
        _ => Err(PyValueError::new_err(
            "mode must be 'compatible' or 'strict'",
        )),
    }
}

fn parse_score_owned(
    data: &[u8],
    limits: Option<&ScoreParseLimits>,
    mode: &str,
) -> PyResult<Score> {
    let options = ScoreParseOptions {
        limits: limits.map_or(CoreScoreParseLimits::DEFAULT, |limits| limits.inner),
        mode: parse_score_mode(mode)?,
    };
    parse_score_smf_with_options(data, options)
        .map(|inner| Score { inner })
        .map_err(|error| PyValueError::new_err(error.to_string()))
}

fn parse_score_unlimited_owned(data: &[u8]) -> PyResult<Score> {
    parse_score_smf(data)
        .map(|inner| Score { inner })
        .map_err(|error| PyValueError::new_err(error.to_string()))
}

/// Parse an in-memory Standard MIDI File into a compact Rust-owned arena.
#[pyfunction]
fn parse(data: &[u8]) -> PyResult<MidiFile> {
    parse_owned(data)
}

/// Parse an in-memory SMF into a native tick score with finite resource limits.
///
/// `limits=None` selects finite core defaults. `mode` is either `compatible`
/// (the default) or `strict`.
#[pyfunction]
#[pyo3(signature = (data, *, limits = None, mode = "compatible"))]
#[allow(clippy::needless_pass_by_value)] // PyO3 extracts an owned PyRef.
fn parse_score(
    data: &[u8],
    limits: Option<PyRef<'_, ScoreParseLimits>>,
    mode: &str,
) -> PyResult<Score> {
    parse_score_owned(data, limits.as_deref(), mode)
}

/// Parse trusted input with the legacy unlimited score parser.
///
/// This bypasses every logical resource ceiling and is unsafe for hostile
/// bytes. Use [`parse_score`] for all untrusted input.
#[pyfunction]
fn parse_score_unlimited(data: &[u8]) -> PyResult<Score> {
    parse_score_unlimited_owned(data)
}

/// Scan an in-memory Standard MIDI File without materializing event objects.
#[pyfunction]
fn scan(data: &[u8]) -> PyResult<ScanSummary> {
    scan_smf(data)
        .map(Into::into)
        .map_err(|error| PyValueError::new_err(error.to_string()))
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<ScanSummary>()?;
    module.add_class::<MidiFile>()?;
    module.add_class::<ScoreParseLimits>()?;
    module.add_class::<Score>()?;
    module.add_function(wrap_pyfunction!(scan, module)?)?;
    module.add_function(wrap_pyfunction!(parse, module)?)?;
    module.add_function(wrap_pyfunction!(parse_score, module)?)?;
    module.add_function(wrap_pyfunction!(parse_score_unlimited, module)?)?;
    Ok(())
}
