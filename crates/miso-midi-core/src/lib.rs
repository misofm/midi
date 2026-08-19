//! High-performance primitives for Standard MIDI File (SMF) data.
//!
//! The first vertical slice intentionally returns a compact scan summary. It
//! exercises every event boundary without imposing an owned object model on
//! downstream SDKs. Borrowed event views and an owned arena can share this
//! decoder in later milestones.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

#[cfg(all(feature = "alloc", not(feature = "std")))]
use core::cell::OnceCell;
use core::fmt;
#[cfg(all(feature = "alloc", feature = "std"))]
use std::sync::OnceLock;

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::boxed::Box;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

const HEADER_CHUNK: [u8; 4] = *b"MThd";
const TRACK_CHUNK: [u8; 4] = *b"MTrk";

/// The SMF format declared by the file header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum Format {
    /// One synchronous track.
    SingleTrack = 0,
    /// Multiple synchronous tracks.
    Parallel = 1,
    /// Multiple asynchronous sequences.
    Sequential = 2,
}

impl TryFrom<u16> for Format {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::SingleTrack),
            1 => Ok(Self::Parallel),
            2 => Ok(Self::Sequential),
            _ => Err(()),
        }
    }
}

/// Header fields needed by both borrowed and owned representations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub format: Format,
    pub track_count: u16,
    /// Raw SMF division field. Its high bit selects metrical or SMPTE timing.
    pub division: u16,
}

/// Explicit resource ceilings for parsing an untrusted tick score.
///
/// These limits apply only to [`parse_score_smf_with_limits`].
/// [`parse_score_smf`] intentionally remains the trusted-input, unlimited
/// fast path for now, and can therefore exhaust memory on hostile input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScoreParseLimits {
    /// Maximum input slice length accepted before header parsing.
    pub max_input_bytes: usize,
    /// Maximum source `MTrk` count declared in the header.
    pub max_source_tracks: u16,
    /// Maximum declared byte length of one source `MTrk` chunk.
    pub max_track_bytes: usize,
    /// Maximum physical events decoded across all source tracks.
    pub max_events: usize,
    /// Maximum note-on events, including dangling note-ons discarded at EOF.
    pub max_note_starts: usize,
    /// Maximum retained, normalized UTF-8 text bytes.
    pub max_text_bytes: usize,
}

impl ScoreParseLimits {
    /// Finite defaults for untrusted score parsing.
    pub const DEFAULT: Self = Self {
        max_input_bytes: 64 * 1024 * 1024,
        max_source_tracks: 4_096,
        max_track_bytes: 16 * 1024 * 1024,
        max_events: 2_000_000,
        max_note_starts: 1_000_000,
        max_text_bytes: 16 * 1024 * 1024,
    };

    /// No logical resource ceilings.
    ///
    /// This is equivalent to the trusted-input behavior of
    /// [`parse_score_smf`], not a safe policy for arbitrary input.
    pub const UNLIMITED: Self = Self {
        max_input_bytes: usize::MAX,
        max_source_tracks: u16::MAX,
        max_track_bytes: usize::MAX,
        max_events: usize::MAX,
        max_note_starts: usize::MAX,
        max_text_bytes: usize::MAX,
    };
}

impl Default for ScoreParseLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Grammar policy used by [`parse_score_smf_with_options`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScoreParseMode {
    /// Finite limits with permissive, sequencer-compatible track framing.
    Compatible,
    /// Finite limits plus strict EOT, division, and trailing-byte validation.
    Strict,
}

/// Bounded score-parser configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScoreParseOptions {
    pub limits: ScoreParseLimits,
    pub mode: ScoreParseMode,
}

impl Default for ScoreParseOptions {
    fn default() -> Self {
        Self {
            limits: ScoreParseLimits::DEFAULT,
            mode: ScoreParseMode::Compatible,
        }
    }
}

/// The resource whose configured score-parse ceiling was exceeded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScoreResource {
    InputBytes,
    SourceTracks,
    TrackBytes,
    Events,
    NoteStarts,
    TextBytes,
}

impl ScoreResource {
    const fn description(self) -> &'static str {
        match self {
            Self::InputBytes => "input bytes",
            Self::SourceTracks => "source tracks",
            Self::TrackBytes => "track bytes",
            Self::Events => "events",
            Self::NoteStarts => "note starts",
            Self::TextBytes => "normalized text bytes",
        }
    }
}

/// Counts produced by a complete structural scan of an SMF byte slice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanSummary {
    pub header: Header,
    pub events: u64,
    pub channel_events: u64,
    pub meta_events: u64,
    pub sysex_events: u64,
    /// Variable-sized meta and `SysEx` bytes that an owned arena must retain.
    pub payload_bytes: u64,
    pub max_delta_ticks: u32,
    pub bytes_consumed: usize,
    pub trailing_bytes: usize,
}

/// Stable, machine-matchable parse failure categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseErrorKind {
    UnexpectedEnd,
    ExpectedChunk {
        expected: [u8; 4],
        actual: [u8; 4],
    },
    HeaderTooShort(u32),
    InvalidFormat(u16),
    VariableLengthQuantityTooLong,
    RunningStatusMissing,
    InvalidDataByte(u8),
    InvalidStatus(u8),
    /// A recognised meta event did not have the required payload length.
    InvalidMetaEvent {
        meta_type: u8,
        length: u32,
    },
    /// A time-signature denominator exponent cannot be represented as `u64`.
    InvalidTimeSignatureDenominator(u8),
    /// Adding a delta time to an absolute tick would overflow `u64`.
    TickOverflow,
    SizeOverflow,
    /// A configured [`ScoreParseLimits`] ceiling was exceeded.
    ResourceLimitExceeded {
        resource: ScoreResource,
        limit: usize,
    },
    /// Strict score parsing requires a positive metrical ticks-per-quarter value.
    InvalidTicksPerQuarter,
    /// Strict score parsing found an unsupported SMPTE division encoding.
    InvalidSmpteDivision {
        frames_per_second: i8,
        ticks_per_frame: u8,
    },
    /// Strict score parsing requires an End-of-Track meta event with no payload.
    InvalidEndOfTrackLength(u32),
    /// Strict score parsing reached a declared track boundary without End-of-Track.
    MissingEndOfTrack,
    /// Strict score parsing found bytes after End-of-Track in one declared track.
    EventAfterEndOfTrack,
    /// Strict score parsing rejects bytes after all declared tracks.
    TrailingBytes,
}

/// A parse failure with an absolute byte offset into the source buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParseError {
    pub offset: usize,
    pub kind: ParseErrorKind,
}

impl ParseError {
    const fn at(offset: usize, kind: ParseErrorKind) -> Self {
        Self { offset, kind }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SMF parse error at byte {}: ", self.offset)?;
        match self.kind {
            ParseErrorKind::UnexpectedEnd => f.write_str("unexpected end of input"),
            ParseErrorKind::ExpectedChunk { expected, actual } => {
                write!(f, "expected chunk {expected:?}, found {actual:?}")
            }
            ParseErrorKind::HeaderTooShort(size) => {
                write!(f, "header payload is {size} bytes; expected at least 6")
            }
            ParseErrorKind::InvalidFormat(format) => write!(f, "invalid SMF format {format}"),
            ParseErrorKind::VariableLengthQuantityTooLong => {
                f.write_str("variable-length quantity exceeds four bytes")
            }
            ParseErrorKind::RunningStatusMissing => {
                f.write_str("data byte encountered without channel running status")
            }
            ParseErrorKind::InvalidDataByte(byte) => {
                write!(f, "channel data byte 0x{byte:02x} has its status bit set")
            }
            ParseErrorKind::InvalidStatus(status) => {
                write!(f, "status 0x{status:02x} is not valid in an SMF track")
            }
            ParseErrorKind::InvalidMetaEvent { meta_type, length } => {
                write!(
                    f,
                    "meta event 0x{meta_type:02x} has invalid payload length {length}"
                )
            }
            ParseErrorKind::InvalidTimeSignatureDenominator(exponent) => {
                write!(
                    f,
                    "time-signature denominator exponent {exponent} exceeds u64"
                )
            }
            ParseErrorKind::TickOverflow => f.write_str("absolute tick exceeds u64"),
            ParseErrorKind::SizeOverflow => f.write_str("declared size does not fit this target"),
            ParseErrorKind::ResourceLimitExceeded { resource, limit } => {
                write!(
                    f,
                    "score parse limit exceeded: {} (limit {limit})",
                    resource.description()
                )
            }
            ParseErrorKind::InvalidTicksPerQuarter => {
                f.write_str("metrical division has zero ticks per quarter")
            }
            ParseErrorKind::InvalidSmpteDivision {
                frames_per_second,
                ticks_per_frame,
            } => write!(
                f,
                "invalid SMPTE division: {frames_per_second} frames per second, {ticks_per_frame} ticks per frame"
            ),
            ParseErrorKind::InvalidEndOfTrackLength(length) => {
                write!(f, "End-of-Track meta event has payload length {length}")
            }
            ParseErrorKind::MissingEndOfTrack => {
                f.write_str("declared track ends without End-of-Track")
            }
            ParseErrorKind::EventAfterEndOfTrack => {
                f.write_str("bytes follow End-of-Track in declared track")
            }
            ParseErrorKind::TrailingBytes => f.write_str("bytes follow declared SMF tracks"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseError {}

/// Decode every event boundary in a Standard MIDI File without allocation.
///
/// This validates chunk bounds, four-byte VLQs, running status, channel data
/// bytes, and the structural lengths of meta and system-exclusive events.
/// Bytes after the declared tracks are reported rather than rejected.
///
/// # Errors
///
/// Returns [`ParseError`] at the first structurally invalid or truncated byte.
pub fn scan_smf(data: &[u8]) -> Result<ScanSummary, ParseError> {
    let mut cursor = Cursor::new(data, 0);
    let header_tag = cursor.read_tag()?;
    if header_tag != HEADER_CHUNK {
        return Err(ParseError::at(
            0,
            ParseErrorKind::ExpectedChunk {
                expected: HEADER_CHUNK,
                actual: header_tag,
            },
        ));
    }

    let header_size_offset = cursor.absolute_offset();
    let header_size = cursor.read_u32()?;
    if header_size < 6 {
        return Err(ParseError::at(
            header_size_offset,
            ParseErrorKind::HeaderTooShort(header_size),
        ));
    }
    let header_size =
        usize::try_from(header_size).map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
    let mut header_cursor = cursor.take_cursor(header_size)?;
    let raw_format = header_cursor.read_u16()?;
    let format = Format::try_from(raw_format).map_err(|()| {
        ParseError::at(
            header_cursor.base,
            ParseErrorKind::InvalidFormat(raw_format),
        )
    })?;
    let track_count = header_cursor.read_u16()?;
    let division = header_cursor.read_u16()?;
    let header = Header {
        format,
        track_count,
        division,
    };

    let mut summary = ScanSummary {
        header,
        events: 0,
        channel_events: 0,
        meta_events: 0,
        sysex_events: 0,
        payload_bytes: 0,
        max_delta_ticks: 0,
        bytes_consumed: 0,
        trailing_bytes: 0,
    };

    for _ in 0..track_count {
        let tag_offset = cursor.absolute_offset();
        let tag = cursor.read_tag()?;
        if tag != TRACK_CHUNK {
            return Err(ParseError::at(
                tag_offset,
                ParseErrorKind::ExpectedChunk {
                    expected: TRACK_CHUNK,
                    actual: tag,
                },
            ));
        }
        let track_size = usize::try_from(cursor.read_u32()?)
            .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
        let track = cursor.take_cursor(track_size)?;
        scan_track(track, &mut summary)?;
    }

    summary.bytes_consumed = cursor.absolute_offset();
    summary.trailing_bytes = cursor.remaining();
    Ok(summary)
}

fn scan_track(mut cursor: Cursor<'_>, summary: &mut ScanSummary) -> Result<(), ParseError> {
    let mut running_status = None;

    while cursor.remaining() != 0 {
        let delta = cursor.read_vlq()?;
        summary.max_delta_ticks = summary.max_delta_ticks.max(delta);

        let event_offset = cursor.absolute_offset();
        let first = cursor.read_u8()?;
        let (status, first_data) = if first < 0x80 {
            let status = running_status.ok_or_else(|| {
                ParseError::at(event_offset, ParseErrorKind::RunningStatusMissing)
            })?;
            (status, Some(first))
        } else {
            if first < 0xf0 {
                running_status = Some(first);
            }
            (first, None)
        };

        match status {
            0x80..=0xef => {
                let data_len = match status >> 4 {
                    0x0c | 0x0d => 1,
                    _ => 2,
                };
                scan_channel_data(&mut cursor, first_data, data_len)?;
                summary.channel_events += 1;
            }
            0xff if first_data.is_none() => {
                cursor.read_u8()?;
                let length = usize::try_from(cursor.read_vlq()?)
                    .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
                cursor.skip(length)?;
                summary.payload_bytes = summary
                    .payload_bytes
                    .checked_add(
                        u64::try_from(length)
                            .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?,
                    )
                    .ok_or_else(|| cursor.error(ParseErrorKind::SizeOverflow))?;
                summary.meta_events += 1;
                // SMF SysEx and meta events interrupt channel running status.
                running_status = None;
            }
            0xf0 | 0xf7 if first_data.is_none() => {
                let length = usize::try_from(cursor.read_vlq()?)
                    .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
                cursor.skip(length)?;
                summary.payload_bytes = summary
                    .payload_bytes
                    .checked_add(
                        u64::try_from(length)
                            .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?,
                    )
                    .ok_or_else(|| cursor.error(ParseErrorKind::SizeOverflow))?;
                summary.sysex_events += 1;
                running_status = None;
            }
            _ => {
                return Err(ParseError::at(
                    event_offset,
                    ParseErrorKind::InvalidStatus(status),
                ));
            }
        }
        summary.events += 1;
    }
    Ok(())
}

fn scan_channel_data(
    cursor: &mut Cursor<'_>,
    first_data: Option<u8>,
    data_len: usize,
) -> Result<(), ParseError> {
    let already_read = usize::from(first_data.is_some());
    if let Some(byte) = first_data {
        validate_data_byte(cursor.absolute_offset() - 1, byte)?;
    }
    for _ in already_read..data_len {
        let offset = cursor.absolute_offset();
        let byte = cursor.read_u8()?;
        validate_data_byte(offset, byte)?;
    }
    Ok(())
}

fn validate_data_byte(offset: usize, byte: u8) -> Result<(), ParseError> {
    if byte < 0x80 {
        Ok(())
    } else {
        Err(ParseError::at(
            offset,
            ParseErrorKind::InvalidDataByte(byte),
        ))
    }
}

/// A contiguous event range belonging to one SMF track.
#[cfg(feature = "alloc")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrackRange {
    start: u32,
    len: u32,
}

#[cfg(feature = "alloc")]
impl TrackRange {
    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    #[must_use]
    pub const fn len(self) -> u32 {
        self.len
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// A compact 16-byte event header in an owned SMF arena.
///
/// Channel data is stored inline. Meta and `SysEx` payloads are ranges into the
/// arena's shared byte buffer.
#[cfg(feature = "alloc")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventRecord {
    delta_ticks: u32,
    payload_start: u32,
    payload_len: u32,
    data: [u8; 2],
    status: u8,
    auxiliary: u8,
}

#[cfg(feature = "alloc")]
impl EventRecord {
    #[must_use]
    pub const fn delta_ticks(self) -> u32 {
        self.delta_ticks
    }

    #[must_use]
    pub const fn status(self) -> u8 {
        self.status
    }

    #[must_use]
    pub const fn meta_type(self) -> Option<u8> {
        if self.status == 0xff {
            Some(self.auxiliary)
        } else {
            None
        }
    }

    #[must_use]
    pub const fn is_channel(self) -> bool {
        self.status >= 0x80 && self.status < 0xf0
    }
}

/// A compact, Rust-owned Standard MIDI File representation.
#[cfg(feature = "alloc")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedSmf {
    header: Header,
    tracks: Vec<TrackRange>,
    events: Vec<EventRecord>,
    payloads: Vec<u8>,
    bytes_consumed: usize,
    trailing_bytes: usize,
}

#[cfg(feature = "alloc")]
impl OwnedSmf {
    #[must_use]
    pub const fn header(&self) -> Header {
        self.header
    }

    #[must_use]
    pub fn tracks(&self) -> &[TrackRange] {
        &self.tracks
    }

    #[must_use]
    pub fn events(&self) -> &[EventRecord] {
        &self.events
    }

    #[must_use]
    pub fn track_events(&self, track: usize) -> Option<&[EventRecord]> {
        let range = *self.tracks.get(track)?;
        let start = usize::try_from(range.start).ok()?;
        let end = start.checked_add(usize::try_from(range.len).ok()?)?;
        self.events.get(start..end)
    }

    #[must_use]
    pub fn event_data(&self, index: usize) -> Option<&[u8]> {
        let event = self.events.get(index)?;
        let len = usize::try_from(event.payload_len).ok()?;
        if event.is_channel() {
            event.data.get(..len)
        } else {
            let start = usize::try_from(event.payload_start).ok()?;
            let end = start.checked_add(len)?;
            self.payloads.get(start..end)
        }
    }

    #[must_use]
    pub const fn bytes_consumed(&self) -> usize {
        self.bytes_consumed
    }

    #[must_use]
    pub const fn trailing_bytes(&self) -> usize {
        self.trailing_bytes
    }

    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        self.tracks.capacity() * core::mem::size_of::<TrackRange>()
            + self.events.capacity() * core::mem::size_of::<EventRecord>()
            + self.payloads.capacity()
    }
}

/// Parse an SMF into compact track ranges, 16-byte event headers, and a shared
/// variable-payload arena.
///
/// The current implementation performs a fast validation/count pass first so
/// all three vectors allocate exactly once. A one-pass growth strategy remains
/// a benchmarkable alternative.
///
/// # Errors
///
/// Returns [`ParseError`] at the first structurally invalid or truncated byte.
#[cfg(feature = "alloc")]
pub fn parse_smf(data: &[u8]) -> Result<OwnedSmf, ParseError> {
    let summary = scan_smf(data)?;
    let event_capacity = usize::try_from(summary.events)
        .map_err(|_| ParseError::at(0, ParseErrorKind::SizeOverflow))?;
    let payload_capacity = usize::try_from(summary.payload_bytes)
        .map_err(|_| ParseError::at(0, ParseErrorKind::SizeOverflow))?;

    let mut cursor = Cursor::new(data, 0);
    cursor.skip(4)?;
    let header_size = usize::try_from(cursor.read_u32()?)
        .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
    cursor.skip(header_size)?;

    let mut owned = OwnedSmf {
        header: summary.header,
        tracks: Vec::with_capacity(usize::from(summary.header.track_count)),
        events: Vec::with_capacity(event_capacity),
        payloads: Vec::with_capacity(payload_capacity),
        bytes_consumed: summary.bytes_consumed,
        trailing_bytes: summary.trailing_bytes,
    };

    for _ in 0..summary.header.track_count {
        cursor.skip(4)?;
        let track_size = usize::try_from(cursor.read_u32()?)
            .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
        let track = cursor.take_cursor(track_size)?;
        parse_track(track, &mut owned)?;
    }

    debug_assert_eq!(owned.events.len(), event_capacity);
    debug_assert_eq!(owned.payloads.len(), payload_capacity);
    Ok(owned)
}

#[cfg(feature = "alloc")]
fn parse_track(mut cursor: Cursor<'_>, owned: &mut OwnedSmf) -> Result<(), ParseError> {
    let start = u32::try_from(owned.events.len())
        .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
    let mut running_status = None;

    while cursor.remaining() != 0 {
        let delta_ticks = cursor.read_vlq()?;
        let event_offset = cursor.absolute_offset();
        let first = cursor.read_u8()?;
        let (status, first_data) = if first < 0x80 {
            let status = running_status.ok_or_else(|| {
                ParseError::at(event_offset, ParseErrorKind::RunningStatusMissing)
            })?;
            (status, Some(first))
        } else {
            if first < 0xf0 {
                running_status = Some(first);
            }
            (first, None)
        };

        match status {
            0x80..=0xef => {
                let data_len = match status >> 4 {
                    0x0c | 0x0d => 1,
                    _ => 2,
                };
                let data = read_channel_data(&mut cursor, first_data, data_len)?;
                owned.events.push(EventRecord {
                    delta_ticks,
                    payload_start: 0,
                    payload_len: u32::try_from(data_len)
                        .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?,
                    data,
                    status,
                    auxiliary: 0,
                });
            }
            0xff if first_data.is_none() => {
                let meta_type = cursor.read_u8()?;
                let length = usize::try_from(cursor.read_vlq()?)
                    .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
                let payload = cursor.take(length)?;
                push_variable_event(owned, delta_ticks, status, meta_type, payload, &cursor)?;
                running_status = None;
            }
            0xf0 | 0xf7 if first_data.is_none() => {
                let length = usize::try_from(cursor.read_vlq()?)
                    .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
                let payload = cursor.take(length)?;
                push_variable_event(owned, delta_ticks, status, 0, payload, &cursor)?;
                running_status = None;
            }
            _ => {
                return Err(ParseError::at(
                    event_offset,
                    ParseErrorKind::InvalidStatus(status),
                ));
            }
        }
    }

    let len = u32::try_from(owned.events.len())
        .ok()
        .and_then(|end| end.checked_sub(start))
        .ok_or_else(|| cursor.error(ParseErrorKind::SizeOverflow))?;
    owned.tracks.push(TrackRange { start, len });
    Ok(())
}

#[cfg(feature = "alloc")]
fn read_channel_data(
    cursor: &mut Cursor<'_>,
    first_data: Option<u8>,
    data_len: usize,
) -> Result<[u8; 2], ParseError> {
    let mut data = [0_u8; 2];
    let mut index = 0;
    if let Some(byte) = first_data {
        validate_data_byte(cursor.absolute_offset() - 1, byte)?;
        data[0] = byte;
        index = 1;
    }
    while index < data_len {
        let offset = cursor.absolute_offset();
        let byte = cursor.read_u8()?;
        validate_data_byte(offset, byte)?;
        data[index] = byte;
        index += 1;
    }
    Ok(data)
}

/// Score parsing overwhelmingly sees two-byte channel events. Keep that path
/// scalar and stack-free instead of routing it through the shared variable
/// length decoder used by the structural owned-SMF parser.
#[cfg(feature = "alloc")]
#[allow(clippy::inline_always)]
#[inline(always)]
fn read_score_channel_data(
    cursor: &mut Cursor<'_>,
    first_data: Option<u8>,
    status: u8,
) -> Result<[u8; 2], ParseError> {
    let first = match first_data {
        Some(byte) => {
            validate_data_byte(cursor.absolute_offset() - 1, byte)?;
            byte
        }
        None => read_score_data_byte(cursor)?,
    };
    if matches!(status >> 4, 0x0c | 0x0d) {
        return Ok([first, 0]);
    }
    Ok([first, read_score_data_byte(cursor)?])
}

#[cfg(feature = "alloc")]
#[allow(clippy::inline_always)]
#[inline(always)]
fn read_score_data_byte(cursor: &mut Cursor<'_>) -> Result<u8, ParseError> {
    let offset = cursor.absolute_offset();
    let byte = cursor.read_u8()?;
    validate_data_byte(offset, byte)?;
    Ok(byte)
}

#[cfg(feature = "alloc")]
fn push_variable_event(
    owned: &mut OwnedSmf,
    delta_ticks: u32,
    status: u8,
    auxiliary: u8,
    payload: &[u8],
    cursor: &Cursor<'_>,
) -> Result<(), ParseError> {
    let payload_start = u32::try_from(owned.payloads.len())
        .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
    let payload_len =
        u32::try_from(payload.len()).map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
    owned.payloads.extend_from_slice(payload);
    owned.events.push(EventRecord {
        delta_ticks,
        payload_start,
        payload_len,
        data: [0; 2],
        status,
        auxiliary,
    });
    Ok(())
}

/// A range into one contiguous score column.
///
/// All score columns use `u32` offsets. SMF chunk sizes are themselves
/// `u32`; a score that would exceed this address space is rejected rather than
/// silently truncating an offset.
#[cfg(feature = "alloc")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScoreRange {
    start: u32,
    len: u32,
}

#[cfg(feature = "alloc")]
impl ScoreRange {
    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    #[must_use]
    pub const fn len(self) -> u32 {
        self.len
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// A byte range in [`TickScore::text_data`]. Text is retained in one shared
/// arena so track names, lyrics, and markers do not allocate separately.
#[cfg(feature = "alloc")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextRange {
    start: u32,
    len: u32,
}

#[cfg(feature = "alloc")]
impl TextRange {
    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    #[must_use]
    pub const fn len(self) -> u32 {
        self.len
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// One completed note in a tick score.
#[cfg(feature = "alloc")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickNote {
    time: u64,
    duration: u64,
    pitch: u8,
    velocity: u8,
}

#[cfg(feature = "alloc")]
impl TickNote {
    #[must_use]
    pub const fn time(self) -> u64 {
        self.time
    }

    #[must_use]
    pub const fn duration(self) -> u64 {
        self.duration
    }

    #[must_use]
    pub const fn pitch(self) -> u8 {
        self.pitch
    }

    #[must_use]
    pub const fn velocity(self) -> u8 {
        self.velocity
    }
}

#[cfg(feature = "alloc")]
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NarrowNoteRow([u8; 10]);

#[cfg(feature = "alloc")]
impl NarrowNoteRow {
    const fn new(time: u32, duration: u32, pitch: u8, velocity: u8) -> Self {
        let time = time.to_le_bytes();
        let duration = duration.to_le_bytes();
        Self([
            time[0],
            time[1],
            time[2],
            time[3],
            duration[0],
            duration[1],
            duration[2],
            duration[3],
            pitch,
            velocity,
        ])
    }

    const fn time(self) -> u32 {
        u32::from_le_bytes([self.0[0], self.0[1], self.0[2], self.0[3]])
    }

    const fn duration(self) -> u32 {
        u32::from_le_bytes([self.0[4], self.0[5], self.0[6], self.0[7]])
    }

    const fn pitch(self) -> u8 {
        self.0[8]
    }

    const fn velocity(self) -> u8 {
        self.0[9]
    }

    const fn with_duration(self, duration: u32) -> Self {
        Self::new(self.time(), duration, self.pitch(), self.velocity())
    }

    fn note(self) -> TickNote {
        TickNote {
            time: u64::from(self.time()),
            duration: u64::from(self.duration()),
            pitch: self.pitch(),
            velocity: self.velocity(),
        }
    }
}

#[cfg(feature = "alloc")]
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WideNoteRow([u8; 18]);

#[cfg(feature = "alloc")]
impl WideNoteRow {
    const fn new(time: u64, duration: u64, pitch: u8, velocity: u8) -> Self {
        let time = time.to_le_bytes();
        let duration = duration.to_le_bytes();
        Self([
            time[0],
            time[1],
            time[2],
            time[3],
            time[4],
            time[5],
            time[6],
            time[7],
            duration[0],
            duration[1],
            duration[2],
            duration[3],
            duration[4],
            duration[5],
            duration[6],
            duration[7],
            pitch,
            velocity,
        ])
    }

    const fn time(self) -> u64 {
        u64::from_le_bytes([
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5], self.0[6], self.0[7],
        ])
    }

    const fn duration(self) -> u64 {
        u64::from_le_bytes([
            self.0[8], self.0[9], self.0[10], self.0[11], self.0[12], self.0[13], self.0[14],
            self.0[15],
        ])
    }

    const fn pitch(self) -> u8 {
        self.0[16]
    }

    const fn velocity(self) -> u8 {
        self.0[17]
    }

    const fn with_duration(self, duration: u64) -> Self {
        Self::new(self.time(), duration, self.pitch(), self.velocity())
    }

    fn note(self) -> TickNote {
        TickNote {
            time: self.time(),
            duration: self.duration(),
            pitch: self.pitch(),
            velocity: self.velocity(),
        }
    }
}

#[cfg(feature = "alloc")]
const _: [(); 10] = [(); core::mem::size_of::<NarrowNoteRow>()];
#[cfg(feature = "alloc")]
const _: [(); 18] = [(); core::mem::size_of::<WideNoteRow>()];

// Fixed byte-array rows preserve an exact 10-byte narrow payload while making
// the normal note-on path one safe Vec append. They avoid packed-field access.

#[cfg(feature = "alloc")]
#[derive(Clone, Debug, Eq, PartialEq)]
enum NoteTimingColumns {
    Narrow(Box<[NarrowNoteRow]>),
    Wide(Box<[WideNoteRow]>),
}

#[cfg(feature = "alloc")]
#[derive(Clone, Debug, Eq, PartialEq)]
struct NoteSegment {
    timing: NoteTimingColumns,
}

#[cfg(feature = "alloc")]
impl NoteSegment {
    fn len(&self) -> usize {
        match &self.timing {
            NoteTimingColumns::Narrow(rows) => rows.len(),
            NoteTimingColumns::Wide(rows) => rows.len(),
        }
    }

    fn get(&self, index: usize) -> Option<TickNote> {
        match &self.timing {
            NoteTimingColumns::Narrow(rows) => Some((*rows.get(index)?).note()),
            NoteTimingColumns::Wide(rows) => Some((*rows.get(index)?).note()),
        }
    }

    fn iter(&self) -> TickNoteIter<'_> {
        TickNoteIter {
            segment: self,
            index: 0,
        }
    }

    fn heap_bytes(&self) -> usize {
        match &self.timing {
            NoteTimingColumns::Narrow(timings) => core::mem::size_of_val(&**timings),
            NoteTimingColumns::Wide(timings) => core::mem::size_of_val(&**timings),
        }
    }
}

/// A lightweight view over one track's packed adaptive note rows.
///
/// Use [`Self::iter`] to obtain complete [`TickNote`] values without
/// materializing the score-wide legacy note cache.
#[cfg(feature = "alloc")]
#[derive(Clone, Copy, Debug)]
pub struct TickNoteView<'a> {
    segment: &'a NoteSegment,
}

#[cfg(feature = "alloc")]
impl<'a> TickNoteView<'a> {
    #[must_use]
    pub fn len(self) -> usize {
        self.segment.len()
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn iter(self) -> TickNoteIter<'a> {
        self.segment.iter()
    }
}

#[cfg(feature = "alloc")]
impl<'a> IntoIterator for TickNoteView<'a> {
    type Item = TickNote;
    type IntoIter = TickNoteIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over complete note values reconstructed from packed rows.
#[cfg(feature = "alloc")]
#[derive(Clone, Debug)]
pub struct TickNoteIter<'a> {
    segment: &'a NoteSegment,
    index: usize,
}

#[cfg(feature = "alloc")]
impl Iterator for TickNoteIter<'_> {
    type Item = TickNote;

    fn next(&mut self) -> Option<Self::Item> {
        let note = self.segment.get(self.index)?;
        self.index += 1;
        Some(note)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.segment.len().saturating_sub(self.index);
        (remaining, Some(remaining))
    }
}

#[cfg(feature = "alloc")]
impl ExactSizeIterator for TickNoteIter<'_> {}

/// A MIDI control-change event in a tick score.
#[cfg(feature = "alloc")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickControlChange {
    row: [u8; 10],
}

#[cfg(feature = "alloc")]
impl TickControlChange {
    const fn new(time: u64, number: u8, value: u8) -> Self {
        let time = time.to_le_bytes();
        Self {
            row: [
                time[0], time[1], time[2], time[3], time[4], time[5], time[6], time[7], number,
                value,
            ],
        }
    }

    #[must_use]
    pub const fn time(self) -> u64 {
        u64::from_le_bytes([
            self.row[0],
            self.row[1],
            self.row[2],
            self.row[3],
            self.row[4],
            self.row[5],
            self.row[6],
            self.row[7],
        ])
    }

    #[must_use]
    pub const fn number(self) -> u8 {
        self.row[8]
    }

    #[must_use]
    pub const fn value(self) -> u8 {
        self.row[9]
    }
}

#[cfg(feature = "alloc")]
const _: [(); 10] = [(); core::mem::size_of::<TickControlChange>()];

/// A centered 14-bit MIDI pitch bend in a tick score.
#[cfg(feature = "alloc")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickPitchBend {
    time: u64,
    value: i16,
}

#[cfg(feature = "alloc")]
impl TickPitchBend {
    #[must_use]
    pub const fn time(self) -> u64 {
        self.time
    }

    /// The raw 14-bit bend value centred around zero (`-8192..=8191`).
    #[must_use]
    pub const fn value(self) -> i16 {
        self.value
    }
}

/// A completed sustain-pedal interval in a tick score.
#[cfg(feature = "alloc")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickPedal {
    time: u64,
    duration: u64,
}

#[cfg(feature = "alloc")]
impl TickPedal {
    #[must_use]
    pub const fn time(self) -> u64 {
        self.time
    }

    #[must_use]
    pub const fn duration(self) -> u64 {
        self.duration
    }
}

/// A timestamped reference into [`TickScore::text_data`].
#[cfg(feature = "alloc")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickText {
    time: u64,
    text: TextRange,
}

#[cfg(feature = "alloc")]
impl TickText {
    #[must_use]
    pub const fn time(self) -> u64 {
        self.time
    }

    #[must_use]
    pub const fn text(self) -> TextRange {
        self.text
    }
}

/// A global tempo event expressed as microseconds per quarter note.
#[cfg(feature = "alloc")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickTempo {
    time: u64,
    microseconds_per_quarter: u32,
}

#[cfg(feature = "alloc")]
impl TickTempo {
    #[must_use]
    pub const fn time(self) -> u64 {
        self.time
    }

    #[must_use]
    pub const fn microseconds_per_quarter(self) -> u32 {
        self.microseconds_per_quarter
    }
}

/// A global time signature whose denominator is the actual power of two.
#[cfg(feature = "alloc")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickTimeSignature {
    time: u64,
    numerator: u8,
    denominator: u64,
}

#[cfg(feature = "alloc")]
impl TickTimeSignature {
    #[must_use]
    pub const fn time(self) -> u64 {
        self.time
    }

    #[must_use]
    pub const fn numerator(self) -> u8 {
        self.numerator
    }

    #[must_use]
    pub const fn denominator(self) -> u64 {
        self.denominator
    }
}

/// A global MIDI key signature.
#[cfg(feature = "alloc")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickKeySignature {
    time: u64,
    key: i8,
    tonality: u8,
}

#[cfg(feature = "alloc")]
impl TickKeySignature {
    #[must_use]
    pub const fn time(self) -> u64 {
        self.time
    }

    #[must_use]
    pub const fn key(self) -> i8 {
        self.key
    }

    /// `0` for major and `1` for minor, as encoded by SMF.
    #[must_use]
    pub const fn tonality(self) -> u8 {
        self.tonality
    }
}

/// One channel/program group emitted from a source SMF track.
#[cfg(feature = "alloc")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickTrack {
    source_track: u16,
    channel: u8,
    program: u8,
    is_drum: bool,
    name: TextRange,
    notes: ScoreRange,
    controls: ScoreRange,
    pitch_bends: ScoreRange,
    pedals: ScoreRange,
    lyrics: ScoreRange,
}

#[cfg(feature = "alloc")]
impl TickTrack {
    #[must_use]
    pub const fn source_track(self) -> u16 {
        self.source_track
    }

    #[must_use]
    pub const fn channel(self) -> u8 {
        self.channel
    }

    #[must_use]
    pub const fn program(self) -> u8 {
        self.program
    }

    #[must_use]
    pub const fn is_drum(self) -> bool {
        self.is_drum
    }

    #[must_use]
    pub const fn name(self) -> TextRange {
        self.name
    }

    #[must_use]
    pub const fn notes(self) -> ScoreRange {
        self.notes
    }

    #[must_use]
    pub const fn controls(self) -> ScoreRange {
        self.controls
    }

    #[must_use]
    pub const fn pitch_bends(self) -> ScoreRange {
        self.pitch_bends
    }

    #[must_use]
    pub const fn pedals(self) -> ScoreRange {
        self.pedals
    }

    #[must_use]
    pub const fn lyrics(self) -> ScoreRange {
        self.lyrics
    }
}

/// A compact owned tick-score arena built directly from an SMF byte slice.
///
/// Notes are retained in typed per-track columns, while the smaller event
/// categories remain contiguous. A [`TickTrack`] stores logical ranges in a
/// stable flattened order; the global note slice is materialized only when its
/// getter is requested. All text bytes share one payload arena.
/// Absolute tick values are `u64`; a file whose cumulative delta would exceed
/// that range returns [`ParseErrorKind::TickOverflow`].
#[cfg(feature = "alloc")]
#[derive(Clone, Debug)]
pub struct TickScore {
    header: Header,
    tracks: Vec<TickTrack>,
    // Notes are retained by output track in adaptive packed rows and flattened
    // only if the legacy global slice getter is explicitly requested. This
    // keeps parse-time construction free of the former per-group-to-global
    // note copy and avoids AoS padding in retained narrow-tick scores.
    note_segments: Vec<NoteSegment>,
    notes: NoteCache,
    note_count: usize,
    controls: Vec<TickControlChange>,
    pitch_bends: Vec<TickPitchBend>,
    pedals: Vec<TickPedal>,
    lyrics: Vec<TickText>,
    tempos: Vec<TickTempo>,
    time_signatures: Vec<TickTimeSignature>,
    key_signatures: Vec<TickKeySignature>,
    markers: Vec<TickText>,
    text_data: Vec<u8>,
    bytes_consumed: usize,
    trailing_bytes: usize,
}

#[cfg(feature = "alloc")]
impl PartialEq for TickScore {
    fn eq(&self, other: &Self) -> bool {
        self.header == other.header
            && self.tracks == other.tracks
            && self.note_segments == other.note_segments
            && self.note_count == other.note_count
            && self.controls == other.controls
            && self.pitch_bends == other.pitch_bends
            && self.pedals == other.pedals
            && self.lyrics == other.lyrics
            && self.tempos == other.tempos
            && self.time_signatures == other.time_signatures
            && self.key_signatures == other.key_signatures
            && self.markers == other.markers
            && self.text_data == other.text_data
            && self.bytes_consumed == other.bytes_consumed
            && self.trailing_bytes == other.trailing_bytes
    }
}

#[cfg(feature = "alloc")]
impl Eq for TickScore {}

#[cfg(feature = "alloc")]
impl TickScore {
    #[must_use]
    pub const fn header(&self) -> Header {
        self.header
    }

    /// The raw SMF division field. For metrical SMF files this is TPQ.
    #[must_use]
    pub const fn ticks_per_quarter(&self) -> u16 {
        self.header.division
    }

    #[must_use]
    pub fn tracks(&self) -> &[TickTrack] {
        &self.tracks
    }

    /// Number of completed notes without materializing the lazy global note
    /// column.
    #[must_use]
    pub const fn note_count(&self) -> usize {
        self.note_count
    }

    /// Lazily materializes all per-track note columns into the legacy global
    /// array-of-structs slice.
    #[must_use]
    pub fn notes(&self) -> &[TickNote] {
        self.notes.get_or_init(|| {
            let mut notes = Vec::with_capacity(self.note_count);
            for segment in &self.note_segments {
                notes.extend(segment.iter());
            }
            notes
        })
    }

    #[must_use]
    pub fn controls(&self) -> &[TickControlChange] {
        &self.controls
    }

    #[must_use]
    pub fn pitch_bends(&self) -> &[TickPitchBend] {
        &self.pitch_bends
    }

    #[must_use]
    pub fn pedals(&self) -> &[TickPedal] {
        &self.pedals
    }

    #[must_use]
    pub fn lyrics(&self) -> &[TickText] {
        &self.lyrics
    }

    #[must_use]
    pub fn tempos(&self) -> &[TickTempo] {
        &self.tempos
    }

    #[must_use]
    pub fn time_signatures(&self) -> &[TickTimeSignature] {
        &self.time_signatures
    }

    #[must_use]
    pub fn key_signatures(&self) -> &[TickKeySignature] {
        &self.key_signatures
    }

    #[must_use]
    pub fn markers(&self) -> &[TickText] {
        &self.markers
    }

    #[must_use]
    pub fn text_data(&self) -> &[u8] {
        &self.text_data
    }

    #[must_use]
    pub fn text(&self, range: TextRange) -> Option<&[u8]> {
        let start = usize::try_from(range.start).ok()?;
        let end = start.checked_add(usize::try_from(range.len).ok()?)?;
        self.text_data.get(start..end)
    }

    /// Returns a lightweight packed-row view of one track's notes without
    /// materializing the legacy global note cache.
    #[must_use]
    pub fn track_notes(&self, track: usize) -> Option<TickNoteView<'_>> {
        self.tracks.get(track)?;
        Some(TickNoteView {
            segment: self.note_segments.get(track)?,
        })
    }

    #[must_use]
    pub fn track_controls(&self, track: usize) -> Option<&[TickControlChange]> {
        self.controls
            .get(range_to_usize(self.tracks.get(track)?.controls)?)
    }

    #[must_use]
    pub fn track_pitch_bends(&self, track: usize) -> Option<&[TickPitchBend]> {
        self.pitch_bends
            .get(range_to_usize(self.tracks.get(track)?.pitch_bends)?)
    }

    #[must_use]
    pub fn track_pedals(&self, track: usize) -> Option<&[TickPedal]> {
        self.pedals
            .get(range_to_usize(self.tracks.get(track)?.pedals)?)
    }

    #[must_use]
    pub fn track_lyrics(&self, track: usize) -> Option<&[TickText]> {
        self.lyrics
            .get(range_to_usize(self.tracks.get(track)?.lyrics)?)
    }

    #[must_use]
    pub const fn bytes_consumed(&self) -> usize {
        self.bytes_consumed
    }

    #[must_use]
    pub const fn trailing_bytes(&self) -> usize {
        self.trailing_bytes
    }

    /// Retained bytes in arena allocations, including exact boxed note-column
    /// payloads and the lazy legacy note cache when materialized.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        self.tracks.capacity() * core::mem::size_of::<TickTrack>()
            + self.note_segments.capacity() * core::mem::size_of::<NoteSegment>()
            + self
                .note_segments
                .iter()
                .map(NoteSegment::heap_bytes)
                .sum::<usize>()
            + self.notes.get().map_or(0, |notes| {
                notes.capacity() * core::mem::size_of::<TickNote>()
            })
            + self.controls.capacity() * core::mem::size_of::<TickControlChange>()
            + self.pitch_bends.capacity() * core::mem::size_of::<TickPitchBend>()
            + self.pedals.capacity() * core::mem::size_of::<TickPedal>()
            + self.lyrics.capacity() * core::mem::size_of::<TickText>()
            + self.tempos.capacity() * core::mem::size_of::<TickTempo>()
            + self.time_signatures.capacity() * core::mem::size_of::<TickTimeSignature>()
            + self.key_signatures.capacity() * core::mem::size_of::<TickKeySignature>()
            + self.markers.capacity() * core::mem::size_of::<TickText>()
            + self.text_data.capacity()
    }

    fn push_text<P: ScoreParsePolicy>(
        &mut self,
        text: &[u8],
        event_offset: usize,
        budget: &P,
        cursor: &Cursor<'_>,
    ) -> Result<TextRange, ParseError> {
        let start = u32::try_from(self.text_data.len())
            .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
        let mut remaining = text;
        // Keep the overwhelmingly common valid-UTF-8 path as one bulk append,
        // and normalize malformed source text with replacement characters.
        loop {
            match core::str::from_utf8(remaining) {
                Ok(_) => {
                    budget.check_text_append(
                        self.text_data.len(),
                        remaining.len(),
                        event_offset,
                    )?;
                    self.text_data.extend_from_slice(remaining);
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    budget.check_text_append(self.text_data.len(), valid, event_offset)?;
                    self.text_data.extend_from_slice(&remaining[..valid]);
                    budget.check_text_append(
                        self.text_data.len(),
                        "\u{fffd}".len(),
                        event_offset,
                    )?;
                    self.text_data.extend_from_slice("\u{fffd}".as_bytes());
                    let invalid = error.error_len().unwrap_or(remaining.len() - valid);
                    remaining = &remaining[valid + invalid..];
                }
            }
        }
        let end = u32::try_from(self.text_data.len())
            .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
        let len = end
            .checked_sub(start)
            .ok_or_else(|| cursor.error(ParseErrorKind::SizeOverflow))?;
        Ok(TextRange { start, len })
    }
}

#[cfg(feature = "alloc")]
fn range_to_usize(range: ScoreRange) -> Option<core::ops::Range<usize>> {
    let start = usize::try_from(range.start).ok()?;
    let end = start.checked_add(usize::try_from(range.len).ok()?)?;
    Some(start..end)
}

#[cfg(feature = "alloc")]
const SCORE_GROUP_SLOTS: usize = 16 * 128;
#[cfg(feature = "alloc")]
const NO_GROUP: u16 = u16::MAX;
#[cfg(feature = "alloc")]
const NO_OPEN_NOTE: u32 = u32::MAX;
#[cfg(feature = "alloc")]
const NO_OVERFLOW_QUEUE: u16 = u16::MAX;

#[cfg(feature = "alloc")]
trait ScoreParsePolicy {
    fn check_input(&self, bytes: usize) -> Result<(), ParseError>;
    fn check_source_tracks(&self, count: u16, offset: usize) -> Result<(), ParseError>;
    fn check_track_bytes(&self, bytes: usize, offset: usize) -> Result<(), ParseError>;
    fn charge_event(&mut self, offset: usize) -> Result<(), ParseError>;
    fn charge_note_start(&mut self, offset: usize) -> Result<(), ParseError>;
    fn check_text_append(
        &self,
        current: usize,
        appended: usize,
        offset: usize,
    ) -> Result<(), ParseError>;
}

/// Zero-sized policy for the trusted parser. It makes the unlimited decoder's
/// no-op budget explicit rather than relying on a const-generic branch to be
/// optimized away.
#[cfg(feature = "alloc")]
struct UnlimitedScoreParsePolicy;

#[cfg(feature = "alloc")]
impl ScoreParsePolicy for UnlimitedScoreParsePolicy {
    #[inline]
    fn check_input(&self, _bytes: usize) -> Result<(), ParseError> {
        Ok(())
    }

    #[inline]
    fn check_source_tracks(&self, _count: u16, _offset: usize) -> Result<(), ParseError> {
        Ok(())
    }

    #[inline]
    fn check_track_bytes(&self, _bytes: usize, _offset: usize) -> Result<(), ParseError> {
        Ok(())
    }

    #[inline]
    fn charge_event(&mut self, _offset: usize) -> Result<(), ParseError> {
        Ok(())
    }

    #[inline]
    fn charge_note_start(&mut self, _offset: usize) -> Result<(), ParseError> {
        Ok(())
    }

    #[inline]
    fn check_text_append(
        &self,
        _current: usize,
        _appended: usize,
        _offset: usize,
    ) -> Result<(), ParseError> {
        Ok(())
    }
}

/// Parser-local counters for the bounded decoder.
#[cfg(feature = "alloc")]
struct LimitedScoreParsePolicy {
    limits: ScoreParseLimits,
    events: usize,
    note_starts: usize,
}

#[cfg(feature = "alloc")]
impl LimitedScoreParsePolicy {
    const fn new(limits: ScoreParseLimits) -> Self {
        Self {
            limits,
            events: 0,
            note_starts: 0,
        }
    }
}

#[cfg(feature = "alloc")]
impl ScoreParsePolicy for LimitedScoreParsePolicy {
    #[inline]
    fn check_input(&self, bytes: usize) -> Result<(), ParseError> {
        if bytes > self.limits.max_input_bytes {
            return Err(ParseError::at(
                0,
                ParseErrorKind::ResourceLimitExceeded {
                    resource: ScoreResource::InputBytes,
                    limit: self.limits.max_input_bytes,
                },
            ));
        }
        Ok(())
    }

    #[inline]
    fn check_source_tracks(&self, count: u16, offset: usize) -> Result<(), ParseError> {
        if count > self.limits.max_source_tracks {
            return Err(ParseError::at(
                offset,
                ParseErrorKind::ResourceLimitExceeded {
                    resource: ScoreResource::SourceTracks,
                    limit: usize::from(self.limits.max_source_tracks),
                },
            ));
        }
        Ok(())
    }

    #[inline]
    fn check_track_bytes(&self, bytes: usize, offset: usize) -> Result<(), ParseError> {
        if bytes > self.limits.max_track_bytes {
            return Err(ParseError::at(
                offset,
                ParseErrorKind::ResourceLimitExceeded {
                    resource: ScoreResource::TrackBytes,
                    limit: self.limits.max_track_bytes,
                },
            ));
        }
        Ok(())
    }

    #[inline]
    fn charge_event(&mut self, offset: usize) -> Result<(), ParseError> {
        if self.events >= self.limits.max_events {
            return Err(ParseError::at(
                offset,
                ParseErrorKind::ResourceLimitExceeded {
                    resource: ScoreResource::Events,
                    limit: self.limits.max_events,
                },
            ));
        }
        self.events += 1;
        Ok(())
    }

    #[inline]
    fn charge_note_start(&mut self, offset: usize) -> Result<(), ParseError> {
        if self.note_starts >= self.limits.max_note_starts {
            return Err(ParseError::at(
                offset,
                ParseErrorKind::ResourceLimitExceeded {
                    resource: ScoreResource::NoteStarts,
                    limit: self.limits.max_note_starts,
                },
            ));
        }
        self.note_starts += 1;
        Ok(())
    }

    #[inline]
    fn check_text_append(
        &self,
        current: usize,
        appended: usize,
        offset: usize,
    ) -> Result<(), ParseError> {
        if current > self.limits.max_text_bytes || appended > self.limits.max_text_bytes - current {
            return Err(ParseError::at(
                offset,
                ParseErrorKind::ResourceLimitExceeded {
                    resource: ScoreResource::TextBytes,
                    limit: self.limits.max_text_bytes,
                },
            ));
        }
        Ok(())
    }
}

#[cfg(all(feature = "alloc", feature = "std"))]
type NoteCache = OnceLock<Vec<TickNote>>;
#[cfg(all(feature = "alloc", not(feature = "std")))]
type NoteCache = OnceCell<Vec<TickNote>>;

#[cfg(feature = "alloc")]
enum NoteTimingBuilder {
    Narrow(Vec<NarrowNoteRow>),
    Wide(Vec<WideNoteRow>),
}

#[cfg(feature = "alloc")]
impl Default for NoteTimingBuilder {
    fn default() -> Self {
        Self::Narrow(Vec::new())
    }
}

#[cfg(feature = "alloc")]
#[derive(Default)]
struct NoteColumnsBuilder {
    timing: NoteTimingBuilder,
    // The all-ones duration is an in-column dangling marker. These sparse
    // indices distinguish the rare completed duration that is itself all
    // ones, so `u32::MAX` and `u64::MAX` remain valid public values.
    completed_max_duration_indices: Vec<u32>,
}

#[cfg(feature = "alloc")]
impl NoteColumnsBuilder {
    fn len(&self) -> usize {
        match &self.timing {
            NoteTimingBuilder::Narrow(rows) => rows.len(),
            NoteTimingBuilder::Wide(rows) => rows.len(),
        }
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[cfg(test)]
    fn capacity(&self) -> usize {
        match &self.timing {
            NoteTimingBuilder::Narrow(rows) => rows.capacity(),
            NoteTimingBuilder::Wide(rows) => rows.capacity(),
        }
    }

    fn reserve_exact(&mut self, capacity: usize) {
        match &mut self.timing {
            NoteTimingBuilder::Narrow(values) => values.reserve_exact(capacity),
            NoteTimingBuilder::Wide(values) => values.reserve_exact(capacity),
        }
    }

    #[inline]
    fn push(
        &mut self,
        time: u64,
        pitch: u8,
        velocity: u8,
        cursor: &Cursor<'_>,
    ) -> Result<u32, ParseError> {
        let index =
            u32::try_from(self.len()).map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
        if let Ok(time) = u32::try_from(time)
            && let NoteTimingBuilder::Narrow(values) = &mut self.timing
        {
            values.push(NarrowNoteRow::new(time, u32::MAX, pitch, velocity));
            return Ok(index);
        }
        match &mut self.timing {
            NoteTimingBuilder::Narrow(_) => self.push_after_promotion(time, pitch, velocity),
            NoteTimingBuilder::Wide(values) => {
                values.push(WideNoteRow::new(time, u64::MAX, pitch, velocity));
            }
        }
        Ok(index)
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn start(&self, index: u32) -> u64 {
        let index = usize::try_from(index).expect("u32 fits usize");
        match &self.timing {
            NoteTimingBuilder::Narrow(values) => u64::from(values[index].time()),
            NoteTimingBuilder::Wide(values) => values[index].time(),
        }
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn complete(&mut self, index: u32, duration: u64) {
        let index_usize = usize::try_from(index).expect("u32 fits usize");
        if let NoteTimingBuilder::Narrow(values) = &mut self.timing {
            if let Ok(duration) = u32::try_from(duration)
                && duration != u32::MAX
            {
                values[index_usize] = values[index_usize].with_duration(duration);
                return;
            }
            self.complete_narrow_exception(index, index_usize, duration);
            return;
        }

        let NoteTimingBuilder::Wide(values) = &mut self.timing else {
            unreachable!("timing is narrow or wide");
        };
        if duration == u64::MAX {
            self.complete_wide_max(index, index_usize);
        } else {
            values[index_usize] = values[index_usize].with_duration(duration);
        }
    }

    #[cold]
    #[inline(never)]
    fn push_after_promotion(&mut self, time: u64, pitch: u8, velocity: u8) {
        self.promote_to_wide();
        let NoteTimingBuilder::Wide(values) = &mut self.timing else {
            unreachable!("narrow timing was promoted");
        };
        values.push(WideNoteRow::new(time, u64::MAX, pitch, velocity));
    }

    #[cold]
    #[inline(never)]
    fn complete_narrow_exception(&mut self, index: u32, index_usize: usize, duration: u64) {
        if duration == u64::from(u32::MAX) {
            let NoteTimingBuilder::Narrow(values) = &mut self.timing else {
                unreachable!("narrow exception still has narrow timing");
            };
            values[index_usize] = values[index_usize].with_duration(u32::MAX);
            self.completed_max_duration_indices.push(index);
            return;
        }

        self.promote_to_wide();
        let NoteTimingBuilder::Wide(values) = &mut self.timing else {
            unreachable!("narrow timing was promoted");
        };
        values[index_usize] = values[index_usize].with_duration(duration);
        if duration == u64::MAX {
            self.completed_max_duration_indices.push(index);
        }
    }

    #[cold]
    #[inline(never)]
    fn complete_wide_max(&mut self, index: u32, index_usize: usize) {
        let NoteTimingBuilder::Wide(values) = &mut self.timing else {
            unreachable!("wide completion still has wide timing");
        };
        values[index_usize] = values[index_usize].with_duration(u64::MAX);
        self.completed_max_duration_indices.push(index);
    }

    #[cold]
    #[inline(never)]
    fn promote_to_wide(&mut self) {
        let timing = core::mem::take(&mut self.timing);
        let NoteTimingBuilder::Narrow(values) = timing else {
            self.timing = timing;
            return;
        };
        let mut wide_values = Vec::with_capacity(values.capacity());
        for (index, row) in values.into_iter().enumerate() {
            let index = u32::try_from(index).expect("note index fits u32");
            let narrow_duration = row.duration();
            let duration = if narrow_duration == u32::MAX
                && !self.completed_max_duration_indices.contains(&index)
            {
                u64::MAX
            } else {
                u64::from(narrow_duration)
            };
            wide_values.push(WideNoteRow::new(
                u64::from(row.time()),
                duration,
                row.pitch(),
                row.velocity(),
            ));
        }
        self.timing = NoteTimingBuilder::Wide(wide_values);
    }

    fn into_segment(self) -> NoteSegment {
        match self.timing {
            NoteTimingBuilder::Narrow(mut values) => {
                if has_dangling_u32(&values, &self.completed_max_duration_indices) {
                    compact_notes_u32(&mut values, &self.completed_max_duration_indices);
                }
                NoteSegment {
                    timing: NoteTimingColumns::Narrow(values.into_boxed_slice()),
                }
            }
            NoteTimingBuilder::Wide(mut values) => {
                if has_dangling_u64(&values, &self.completed_max_duration_indices) {
                    compact_notes_u64(&mut values, &self.completed_max_duration_indices);
                }
                NoteSegment {
                    timing: NoteTimingColumns::Wide(values.into_boxed_slice()),
                }
            }
        }
    }
}

#[cfg(feature = "alloc")]
fn has_dangling_u32(values: &[NarrowNoteRow], completed_max_duration_indices: &[u32]) -> bool {
    values.iter().enumerate().any(|(index, row)| {
        row.duration() == u32::MAX
            && !completed_max_duration_indices
                .contains(&u32::try_from(index).expect("note index fits u32"))
    })
}

#[cfg(feature = "alloc")]
fn has_dangling_u64(values: &[WideNoteRow], completed_max_duration_indices: &[u32]) -> bool {
    values.iter().enumerate().any(|(index, row)| {
        row.duration() == u64::MAX
            && !completed_max_duration_indices
                .contains(&u32::try_from(index).expect("note index fits u32"))
    })
}

#[cfg(feature = "alloc")]
fn compact_notes_u32(values: &mut Vec<NarrowNoteRow>, completed_max_duration_indices: &[u32]) {
    let mut write = 0;
    for read in 0..values.len() {
        let completed = values[read].duration() != u32::MAX
            || completed_max_duration_indices
                .contains(&u32::try_from(read).expect("note index fits u32"));
        if completed {
            if write != read {
                values[write] = values[read];
            }
            write += 1;
        }
    }
    values.truncate(write);
}

#[cfg(feature = "alloc")]
fn compact_notes_u64(values: &mut Vec<WideNoteRow>, completed_max_duration_indices: &[u32]) {
    let mut write = 0;
    for read in 0..values.len() {
        let completed = values[read].duration() != u64::MAX
            || completed_max_duration_indices
                .contains(&u32::try_from(read).expect("note index fits u32"));
        if completed {
            if write != read {
                values[write] = values[read];
            }
            write += 1;
        }
    }
    values.truncate(write);
}

#[cfg(feature = "alloc")]
#[derive(Default)]
struct ScoreGroupBuilder {
    channel: u8,
    program: u8,
    notes: NoteColumnsBuilder,
    controls: Vec<TickControlChange>,
    pitch_bends: Vec<TickPitchBend>,
    pedals: Vec<TickPedal>,
    lyrics: Vec<TickText>,
}

#[cfg(feature = "alloc")]
impl ScoreGroupBuilder {
    fn new(channel: u8, program: u8) -> Self {
        Self {
            channel,
            program,
            ..Self::default()
        }
    }

    fn is_empty(&self) -> bool {
        self.notes.is_empty()
            && self.controls.is_empty()
            && self.pitch_bends.is_empty()
            && self.pedals.is_empty()
            && self.lyrics.is_empty()
    }
}

/// A queued active note after the primary slot is occupied.
#[cfg(feature = "alloc")]
#[derive(Clone, Copy)]
struct OverflowNote {
    group: u16,
    note: u32,
    next: u32,
}

/// Queue metadata exists only for channel/pitch pairs with overlapping notes.
#[cfg(feature = "alloc")]
struct OverflowQueue {
    head: u32,
    tail: u32,
}

#[cfg(feature = "alloc")]
struct ScoreTrackBuilder {
    programs: [u8; 16],
    groups_by_key: [u16; SCORE_GROUP_SLOTS],
    groups: Vec<ScoreGroupBuilder>,
    active_programs: [u128; 16],
    note_capacity_hint: Option<usize>,
    stragglers: [ScoreGroupBuilder; 16],
    // One direct primary slot per channel/pitch. The packed state carries its
    // note index and common u32 start tick, avoiding a row lookup on note-off.
    open_groups: [u16; SCORE_GROUP_SLOTS],
    open_note_states: [u64; SCORE_GROUP_SLOTS],
    overflow_queues_by_key: [u16; SCORE_GROUP_SLOTS],
    overflow_notes: Vec<OverflowNote>,
    overflow_queues: Vec<OverflowQueue>,
    free_overflow_queues: Vec<u16>,
    pedal_starts: [Option<u64>; 16],
}

#[cfg(feature = "alloc")]
impl ScoreTrackBuilder {
    fn new(track_bytes: usize) -> Self {
        Self {
            programs: [0; 16],
            groups_by_key: [NO_GROUP; SCORE_GROUP_SLOTS],
            groups: Vec::new(),
            active_programs: [0; 16],
            // A completed note-on/note-off pair occupies at least six SMF
            // bytes under running status. Reserve only for the first group
            // that actually receives a note: this avoids early Vec growth on
            // dense tracks without allocating for metadata-only tracks or
            // holding more than 2,048 temporary note rows (36,864 B after a
            // rare wide promotion). Retained boxed columns are exact-length.
            note_capacity_hint: Some(core::cmp::min(track_bytes / 6, 2_048)),
            stragglers: core::array::from_fn(|_| ScoreGroupBuilder::default()),
            open_groups: [NO_GROUP; SCORE_GROUP_SLOTS],
            open_note_states: [0; SCORE_GROUP_SLOTS],
            overflow_queues_by_key: [NO_OVERFLOW_QUEUE; SCORE_GROUP_SLOTS],
            overflow_notes: Vec::new(),
            overflow_queues: Vec::new(),
            free_overflow_queues: Vec::new(),
            pedal_starts: [None; 16],
        }
    }

    fn key(channel: u8, program: u8) -> usize {
        usize::from(channel) * 128 + usize::from(program)
    }

    fn note_key(channel: u8, pitch: u8) -> usize {
        usize::from(channel) * 128 + usize::from(pitch)
    }

    #[inline]
    fn pack_open_note_state(note: u32, time: u64) -> u64 {
        let start = match u32::try_from(time) {
            Ok(start) if start != u32::MAX => start,
            Ok(_) | Err(_) => u32::MAX,
        };
        (u64::from(start) << 32) | u64::from(note)
    }

    #[inline]
    fn open_note_index(state: u64) -> u32 {
        u32::try_from(state & u64::from(u32::MAX)).expect("low 32 bits fit u32")
    }

    #[inline]
    fn open_note_start(state: u64) -> Option<u64> {
        let start = u32::try_from(state >> 32).expect("high 32 bits fit u32");
        (start != u32::MAX).then_some(u64::from(start))
    }

    fn set_program(&mut self, channel: u8, program: u8) {
        self.programs[usize::from(channel)] = program;
    }

    fn group_or_straggler_mut(&mut self, channel: u8) -> &mut ScoreGroupBuilder {
        let program = self.programs[usize::from(channel)];
        let key = Self::key(channel, program);
        let group = self.groups_by_key[key];
        if group == NO_GROUP {
            &mut self.stragglers[usize::from(channel)]
        } else {
            &mut self.groups[usize::from(group)]
        }
    }

    #[inline]
    fn ensure_group(&mut self, channel: u8, cursor: &Cursor<'_>) -> Result<u16, ParseError> {
        let program = self.programs[usize::from(channel)];
        let key = Self::key(channel, program);
        let current = self.groups_by_key[key];
        if current != NO_GROUP {
            return Ok(current);
        }

        self.create_group(channel, program, key, cursor)
    }

    #[cold]
    #[inline(never)]
    fn create_group(
        &mut self,
        channel: u8,
        program: u8,
        key: usize,
        cursor: &Cursor<'_>,
    ) -> Result<u16, ParseError> {
        let index = u16::try_from(self.groups.len())
            .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
        let mut group = ScoreGroupBuilder::new(channel, program);
        let straggler = core::mem::take(&mut self.stragglers[usize::from(channel)]);
        group.controls = straggler.controls;
        group.pitch_bends = straggler.pitch_bends;
        group.pedals = straggler.pedals;
        group.lyrics = straggler.lyrics;
        self.groups.push(group);
        self.groups_by_key[key] = index;
        self.active_programs[usize::from(channel)] |= 1_u128 << program;
        Ok(index)
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn add_note(
        &mut self,
        channel: u8,
        pitch: u8,
        velocity: u8,
        time: u64,
        cursor: &Cursor<'_>,
    ) -> Result<(), ParseError> {
        let group = self.ensure_group(channel, cursor)?;
        let group_index = usize::from(group);
        if self.groups[group_index].notes.is_empty()
            && let Some(capacity) = self.note_capacity_hint.take()
        {
            self.groups[group_index].notes.reserve_exact(capacity);
        }
        let note = self.groups[group_index]
            .notes
            .push(time, pitch, velocity, cursor)?;
        let key = Self::note_key(channel, pitch);
        if self.open_groups[key] == NO_GROUP {
            self.open_groups[key] = group;
            self.open_note_states[key] = Self::pack_open_note_state(note, time);
            debug_assert_eq!(self.overflow_queues_by_key[key], NO_OVERFLOW_QUEUE);
        } else {
            let index = u32::try_from(self.overflow_notes.len())
                .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
            self.overflow_notes.push(OverflowNote {
                group,
                note,
                next: NO_OPEN_NOTE,
            });
            let queue_index = self.ensure_overflow_queue(key, cursor)?;
            let queue_index = usize::from(queue_index);
            let previous_tail = self.overflow_queues[queue_index].tail;
            if previous_tail == NO_OPEN_NOTE {
                self.overflow_queues[queue_index].head = index;
            } else {
                self.overflow_notes[usize::try_from(previous_tail).expect("u32 fits usize")].next =
                    index;
            }
            self.overflow_queues[queue_index].tail = index;
        }
        Ok(())
    }

    // Perf/disassembly shows this was otherwise emitted as a Result-returning
    // call on every note-off. Its hot, infallible path benefits from forcing
    // the direct-table access into the decoder.
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn end_note(&mut self, channel: u8, pitch: u8, time: u64) {
        let key = Self::note_key(channel, pitch);
        let group = self.open_groups[key];
        if group == NO_GROUP {
            return;
        }
        let state = self.open_note_states[key];
        let note = Self::open_note_index(state);
        let start = Self::open_note_start(state)
            .unwrap_or_else(|| self.groups[usize::from(group)].notes.start(note));
        // Deltas are non-negative and accumulated monotonically before an
        // event is dispatched. An open note therefore cannot start after its
        // note-off in the same source track.
        debug_assert!(time >= start);
        let duration = time - start;
        self.groups[usize::from(group)]
            .notes
            .complete(note, duration);
        let queue_index = self.overflow_queues_by_key[key];
        if queue_index == NO_OVERFLOW_QUEUE {
            self.open_groups[key] = NO_GROUP;
            self.open_note_states[key] = 0;
            return;
        }

        let queue_index_usize = usize::from(queue_index);
        let overflow_head = self.overflow_queues[queue_index_usize].head;
        if overflow_head == NO_OPEN_NOTE {
            self.open_groups[key] = NO_GROUP;
            self.open_note_states[key] = 0;
            self.overflow_queues_by_key[key] = NO_OVERFLOW_QUEUE;
            self.free_overflow_queues.push(queue_index);
            return;
        }
        let next = self.overflow_notes[usize::try_from(overflow_head).expect("u32 fits usize")];
        self.open_groups[key] = next.group;
        let next_time = self.groups[usize::from(next.group)].notes.start(next.note);
        self.open_note_states[key] = Self::pack_open_note_state(next.note, next_time);
        self.overflow_queues[queue_index_usize].head = next.next;
        if next.next == NO_OPEN_NOTE {
            self.overflow_queues[queue_index_usize].tail = NO_OPEN_NOTE;
        }
    }

    fn ensure_overflow_queue(
        &mut self,
        key: usize,
        cursor: &Cursor<'_>,
    ) -> Result<u16, ParseError> {
        let current = self.overflow_queues_by_key[key];
        if current != NO_OVERFLOW_QUEUE {
            return Ok(current);
        }
        let index = if let Some(index) = self.free_overflow_queues.pop() {
            index
        } else {
            let index = u16::try_from(self.overflow_queues.len())
                .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
            self.overflow_queues.push(OverflowQueue {
                head: NO_OPEN_NOTE,
                tail: NO_OPEN_NOTE,
            });
            index
        };
        self.overflow_queues_by_key[key] = index;
        Ok(index)
    }

    fn add_lyric(
        &mut self,
        time: u64,
        text: TextRange,
        cursor: &Cursor<'_>,
    ) -> Result<(), ParseError> {
        // Lyric metadata belongs to the current channel-zero program rather
        // than the preceding channel event.
        let group = self.ensure_group(0, cursor)?;
        self.groups[usize::from(group)]
            .lyrics
            .push(TickText { time, text });
        Ok(())
    }

    fn add_control(&mut self, channel: u8, time: u64, number: u8, value: u8) {
        self.group_or_straggler_mut(channel)
            .controls
            .push(TickControlChange::new(time, number, value));
    }

    fn add_pitch_bend(&mut self, channel: u8, time: u64, value: i16) {
        self.group_or_straggler_mut(channel)
            .pitch_bends
            .push(TickPitchBend { time, value });
    }

    fn handle_pedal(&mut self, channel: u8, time: u64, value: u8) -> Result<(), ParseError> {
        let pedal = &mut self.pedal_starts[usize::from(channel)];
        if value >= 64 {
            if pedal.is_none() {
                *pedal = Some(time);
            }
            return Ok(());
        }
        let Some(start) = pedal.take() else {
            return Ok(());
        };
        let duration = time
            .checked_sub(start)
            .ok_or(ParseError::at(0, ParseErrorKind::TickOverflow))?;
        self.group_or_straggler_mut(channel).pedals.push(TickPedal {
            time: start,
            duration,
        });
        Ok(())
    }

    fn finish(
        mut self,
        source_track: u16,
        name: TextRange,
        score: &mut TickScore,
        cursor: &Cursor<'_>,
    ) -> Result<(), ParseError> {
        // Iterating the fixed per-channel bitsets yields deterministic
        // source-track/channel/program order without allocating, sorting, or
        // sweeping all 2,048 possible group slots.
        for channel in 0_u8..16 {
            let mut programs = self.active_programs[usize::from(channel)];
            while programs != 0 {
                let program =
                    u8::try_from(programs.trailing_zeros()).expect("u128 bit index fits u8");
                programs &= programs - 1;
                let key = Self::key(channel, program);
                let group_index = self.groups_by_key[key];
                debug_assert_ne!(group_index, NO_GROUP);
                let group = core::mem::take(&mut self.groups[usize::from(group_index)]);
                if group.is_empty() {
                    continue;
                }
                let notes = score.push_note_segment(group.notes.into_segment(), cursor)?;
                let controls = append_column(&mut score.controls, &group.controls, cursor)?;
                let pitch_bends =
                    append_column(&mut score.pitch_bends, &group.pitch_bends, cursor)?;
                let pedals = append_column(&mut score.pedals, &group.pedals, cursor)?;
                let lyrics = append_column(&mut score.lyrics, &group.lyrics, cursor)?;
                score.tracks.push(TickTrack {
                    source_track,
                    channel: group.channel,
                    program: group.program,
                    is_drum: group.channel == 9,
                    name,
                    notes,
                    controls,
                    pitch_bends,
                    pedals,
                    lyrics,
                });
            }
        }
        Ok(())
    }
}

#[cfg(feature = "alloc")]
impl TickScore {
    fn push_note_segment(
        &mut self,
        notes: NoteSegment,
        cursor: &Cursor<'_>,
    ) -> Result<ScoreRange, ParseError> {
        let start = u32::try_from(self.note_count)
            .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
        let len =
            u32::try_from(notes.len()).map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| cursor.error(ParseErrorKind::SizeOverflow))?;
        self.note_count =
            usize::try_from(end).map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
        self.note_segments.push(notes);
        Ok(ScoreRange { start, len })
    }
}

#[cfg(feature = "alloc")]
fn append_column<T: Copy>(
    output: &mut Vec<T>,
    input: &[T],
    cursor: &Cursor<'_>,
) -> Result<ScoreRange, ParseError> {
    let start =
        u32::try_from(output.len()).map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
    let len = u32::try_from(input.len()).map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
    let _end = start
        .checked_add(len)
        .ok_or_else(|| cursor.error(ParseErrorKind::SizeOverflow))?;
    output.extend_from_slice(input);
    Ok(ScoreRange { start, len })
}

/// Parse an SMF directly into a compact, tick-based score arena.
///
/// This parser performs no `OwnedSmf` conversion and never materialises a
/// general object graph. It decodes each semantic event once after framing the
/// header and track chunks, with fixed direct-index state for channel/program
/// grouping and FIFO note closure.
///
/// # Errors
///
/// Returns [`ParseError`] at the first structurally invalid byte, malformed
/// recognised score meta event, or overflowing absolute tick.
///
/// # Resource use
///
/// This is the trusted-input unlimited fast path. It intentionally performs
/// no logical resource checks and can exhaust memory on arbitrary input. Use
/// [`parse_score_smf_with_limits`] for untrusted input.
#[cfg(feature = "alloc")]
pub fn parse_score_smf(data: &[u8]) -> Result<TickScore, ParseError> {
    parse_score_smf_impl::<UnlimitedScoreParsePolicy, LegacyScoreGrammar>(
        data,
        UnlimitedScoreParsePolicy,
    )
}

/// Parse an SMF directly into a compact tick score with explicit resource
/// ceilings suitable for untrusted input.
///
/// The checked decoder is separately monomorphized from [`parse_score_smf`],
/// preserving that trusted parser's no-check hot path.
///
/// # Errors
///
/// Returns [`ParseErrorKind::ResourceLimitExceeded`] when a configured
/// [`ScoreParseLimits`] ceiling is reached.
#[cfg(feature = "alloc")]
pub fn parse_score_smf_with_limits(
    data: &[u8],
    limits: ScoreParseLimits,
) -> Result<TickScore, ParseError> {
    parse_score_smf_impl::<LimitedScoreParsePolicy, LegacyScoreGrammar>(
        data,
        LimitedScoreParsePolicy::new(limits),
    )
}

/// Parse an SMF score using finite limits and an explicit grammar policy.
///
/// [`ScoreParseMode::Compatible`] accepts omitted End-of-Track events and
/// ignores the remainder of a declared track after its first End-of-Track.
/// [`ScoreParseMode::Strict`] additionally validates track termination,
/// division encoding, and file trailing bytes.
///
/// # Errors
///
/// Returns [`ParseError`] for malformed SMF data, configured resource limits,
/// or grammar violations selected by [`ScoreParseMode`].
#[cfg(feature = "alloc")]
pub fn parse_score_smf_with_options(
    data: &[u8],
    options: ScoreParseOptions,
) -> Result<TickScore, ParseError> {
    match options.mode {
        ScoreParseMode::Compatible => parse_score_smf_impl::<
            LimitedScoreParsePolicy,
            CompatibleScoreGrammar,
        >(data, LimitedScoreParsePolicy::new(options.limits)),
        ScoreParseMode::Strict => {
            parse_score_smf_impl::<LimitedScoreParsePolicy, StrictScoreGrammar>(
                data,
                LimitedScoreParsePolicy::new(options.limits),
            )
        }
    }
}

#[cfg(feature = "alloc")]
trait ScoreGrammar {
    const STOP_AT_END_OF_TRACK: bool;
    const REQUIRE_END_OF_TRACK: bool;
    const VALIDATE_DIVISION: bool;
    const REJECT_TRAILING_BYTES: bool;
}

#[cfg(feature = "alloc")]
struct LegacyScoreGrammar;

#[cfg(feature = "alloc")]
impl ScoreGrammar for LegacyScoreGrammar {
    const STOP_AT_END_OF_TRACK: bool = false;
    const REQUIRE_END_OF_TRACK: bool = false;
    const VALIDATE_DIVISION: bool = false;
    const REJECT_TRAILING_BYTES: bool = false;
}

#[cfg(feature = "alloc")]
struct CompatibleScoreGrammar;

#[cfg(feature = "alloc")]
impl ScoreGrammar for CompatibleScoreGrammar {
    const STOP_AT_END_OF_TRACK: bool = true;
    const REQUIRE_END_OF_TRACK: bool = false;
    const VALIDATE_DIVISION: bool = false;
    const REJECT_TRAILING_BYTES: bool = false;
}

#[cfg(feature = "alloc")]
struct StrictScoreGrammar;

#[cfg(feature = "alloc")]
impl ScoreGrammar for StrictScoreGrammar {
    const STOP_AT_END_OF_TRACK: bool = true;
    const REQUIRE_END_OF_TRACK: bool = true;
    const VALIDATE_DIVISION: bool = true;
    const REJECT_TRAILING_BYTES: bool = true;
}

#[cfg(feature = "alloc")]
fn parse_score_smf_impl<P: ScoreParsePolicy, G: ScoreGrammar>(
    data: &[u8],
    mut budget: P,
) -> Result<TickScore, ParseError> {
    budget.check_input(data.len())?;
    let mut cursor = Cursor::new(data, 0);
    let header_tag = cursor.read_tag()?;
    if header_tag != HEADER_CHUNK {
        return Err(ParseError::at(
            0,
            ParseErrorKind::ExpectedChunk {
                expected: HEADER_CHUNK,
                actual: header_tag,
            },
        ));
    }
    let header_size_offset = cursor.absolute_offset();
    let header_size = cursor.read_u32()?;
    if header_size < 6 {
        return Err(ParseError::at(
            header_size_offset,
            ParseErrorKind::HeaderTooShort(header_size),
        ));
    }
    let header_size =
        usize::try_from(header_size).map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
    let mut header_cursor = cursor.take_cursor(header_size)?;
    let raw_format = header_cursor.read_u16()?;
    let format = Format::try_from(raw_format).map_err(|()| {
        ParseError::at(
            header_cursor.base,
            ParseErrorKind::InvalidFormat(raw_format),
        )
    })?;
    let track_count_offset = header_cursor.absolute_offset();
    let track_count = header_cursor.read_u16()?;
    budget.check_source_tracks(track_count, track_count_offset)?;
    let division_offset = header_cursor.absolute_offset();
    let division = header_cursor.read_u16()?;
    if G::VALIDATE_DIVISION {
        validate_strict_division(division, division_offset)?;
    }
    let header = Header {
        format,
        track_count,
        division,
    };

    let mut score = TickScore {
        header,
        tracks: Vec::new(),
        note_segments: Vec::new(),
        notes: NoteCache::new(),
        note_count: 0,
        controls: Vec::new(),
        pitch_bends: Vec::new(),
        pedals: Vec::new(),
        lyrics: Vec::new(),
        tempos: Vec::new(),
        time_signatures: Vec::new(),
        key_signatures: Vec::new(),
        markers: Vec::new(),
        text_data: Vec::new(),
        bytes_consumed: 0,
        trailing_bytes: 0,
    };

    for source_track in 0..header.track_count {
        let tag_offset = cursor.absolute_offset();
        let tag = cursor.read_tag()?;
        if tag != TRACK_CHUNK {
            return Err(ParseError::at(
                tag_offset,
                ParseErrorKind::ExpectedChunk {
                    expected: TRACK_CHUNK,
                    actual: tag,
                },
            ));
        }
        let track_size_offset = cursor.absolute_offset();
        let track_size = usize::try_from(cursor.read_u32()?)
            .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
        budget.check_track_bytes(track_size, track_size_offset)?;
        let track = cursor.take_cursor(track_size)?;
        parse_score_track::<P, G>(track, source_track, &mut score, &mut budget)?;
    }

    score.bytes_consumed = cursor.absolute_offset();
    score.trailing_bytes = cursor.remaining();
    if G::REJECT_TRAILING_BYTES && score.trailing_bytes != 0 {
        return Err(ParseError::at(
            score.bytes_consumed,
            ParseErrorKind::TrailingBytes,
        ));
    }
    score.tempos.sort_by_key(|event| event.time);
    score.time_signatures.sort_by_key(|event| event.time);
    score.key_signatures.sort_by_key(|event| event.time);
    score.markers.sort_by_key(|event| event.time);
    Ok(score)
}

#[cfg(feature = "alloc")]
struct ScoreTrackSemantics<'a> {
    builder: &'a mut ScoreTrackBuilder,
    score: &'a mut TickScore,
    name: TextRange,
}

#[cfg(feature = "alloc")]
#[derive(Clone, Copy)]
struct ScoreMetaEvent<'a> {
    meta_type: u8,
    length: u32,
    payload: &'a [u8],
    time: u64,
    event_offset: usize,
}

#[cfg(feature = "alloc")]
impl ScoreTrackSemantics<'_> {
    fn channel<P: ScoreParsePolicy>(
        &mut self,
        status: u8,
        data: [u8; 2],
        time: u64,
        event_offset: usize,
        budget: &mut P,
        cursor: &Cursor<'_>,
    ) -> Result<(), ParseError> {
        let channel = status & 0x0f;
        match status >> 4 {
            0x08 => self.builder.end_note(channel, data[0], time),
            0x09 if data[1] == 0 => self.builder.end_note(channel, data[0], time),
            0x09 => {
                budget.charge_note_start(event_offset)?;
                self.builder
                    .add_note(channel, data[0], data[1], time, cursor)?;
            }
            0x0b => {
                self.builder.add_control(channel, time, data[0], data[1]);
                if data[0] == 64 {
                    self.builder.handle_pedal(channel, time, data[1])?;
                }
            }
            0x0c => self.builder.set_program(channel, data[0]),
            0x0e => {
                let raw = i16::from(data[0]) | (i16::from(data[1]) << 7);
                self.builder.add_pitch_bend(channel, time, raw - 8192);
            }
            _ => {}
        }
        Ok(())
    }

    fn meta<P: ScoreParsePolicy>(
        &mut self,
        event: ScoreMetaEvent<'_>,
        budget: &P,
        cursor: &Cursor<'_>,
    ) -> Result<(), ParseError> {
        let invalid_length = || {
            ParseError::at(
                event.event_offset,
                ParseErrorKind::InvalidMetaEvent {
                    meta_type: event.meta_type,
                    length: event.length,
                },
            )
        };
        match event.meta_type {
            0x03 => {
                self.name =
                    self.score
                        .push_text(event.payload, event.event_offset, budget, cursor)?;
            }
            0x05 if !event.payload.is_empty() => {
                let text =
                    self.score
                        .push_text(event.payload, event.event_offset, budget, cursor)?;
                self.builder.add_lyric(event.time, text, cursor)?;
            }
            0x06 if !event.payload.is_empty() => {
                let text =
                    self.score
                        .push_text(event.payload, event.event_offset, budget, cursor)?;
                self.score.markers.push(TickText {
                    time: event.time,
                    text,
                });
            }
            0x51 => {
                if event.payload.len() != 3 {
                    return Err(invalid_length());
                }
                self.score.tempos.push(TickTempo {
                    time: event.time,
                    microseconds_per_quarter: (u32::from(event.payload[0]) << 16)
                        | (u32::from(event.payload[1]) << 8)
                        | u32::from(event.payload[2]),
                });
            }
            0x58 => {
                if event.payload.len() != 4 {
                    return Err(invalid_length());
                }
                let exponent = event.payload[1];
                let denominator = 1_u64
                    .checked_shl(u32::from(exponent))
                    .ok_or(ParseError::at(
                        event.event_offset,
                        ParseErrorKind::InvalidTimeSignatureDenominator(exponent),
                    ))?;
                self.score.time_signatures.push(TickTimeSignature {
                    time: event.time,
                    numerator: event.payload[0],
                    denominator,
                });
            }
            0x59 => {
                if event.payload.len() != 2 {
                    return Err(invalid_length());
                }
                self.score.key_signatures.push(TickKeySignature {
                    time: event.time,
                    key: i8::from_be_bytes([event.payload[0]]),
                    tonality: event.payload[1],
                });
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(feature = "alloc")]
fn validate_score_meta(
    meta_type: u8,
    length: u32,
    payload: &[u8],
    event_offset: usize,
) -> Result<(), ParseError> {
    let invalid_length = || {
        ParseError::at(
            event_offset,
            ParseErrorKind::InvalidMetaEvent { meta_type, length },
        )
    };
    match meta_type {
        0x51 if payload.len() != 3 => Err(invalid_length()),
        0x58 if payload.len() != 4 => Err(invalid_length()),
        0x58 if 1_u64.checked_shl(u32::from(payload[1])).is_none() => Err(ParseError::at(
            event_offset,
            ParseErrorKind::InvalidTimeSignatureDenominator(payload[1]),
        )),
        0x59 if payload.len() != 2 => Err(invalid_length()),
        _ => Ok(()),
    }
}

#[cfg(feature = "alloc")]
fn handle_end_of_track<P: ScoreParsePolicy, G: ScoreGrammar>(
    length: u32,
    event_offset: usize,
    cursor: &mut Cursor<'_>,
    budget: &mut P,
) -> Result<(), ParseError> {
    if G::REQUIRE_END_OF_TRACK && length != 0 {
        return Err(ParseError::at(
            event_offset,
            ParseErrorKind::InvalidEndOfTrackLength(length),
        ));
    }
    budget.charge_event(event_offset)?;
    // RP-001 requires EOT to be the final track event. In compatible mode
    // intentionally ignore the rest of the declared chunk after first EOT.
    if G::REQUIRE_END_OF_TRACK && cursor.remaining() != 0 {
        return Err(ParseError::at(
            cursor.absolute_offset(),
            ParseErrorKind::EventAfterEndOfTrack,
        ));
    }
    if !G::REQUIRE_END_OF_TRACK {
        cursor.skip(cursor.remaining())?;
    }
    Ok(())
}

#[cfg(feature = "alloc")]
fn validate_strict_division(division: u16, offset: usize) -> Result<(), ParseError> {
    if division & 0x8000 == 0 {
        return (division != 0).then_some(()).ok_or(ParseError::at(
            offset,
            ParseErrorKind::InvalidTicksPerQuarter,
        ));
    }
    let [frame_byte, ticks_per_frame] = division.to_be_bytes();
    let frames_per_second = i8::from_be_bytes([frame_byte]);
    if !matches!(frames_per_second, -24 | -25 | -29 | -30) || ticks_per_frame == 0 {
        return Err(ParseError::at(
            offset,
            ParseErrorKind::InvalidSmpteDivision {
                frames_per_second,
                ticks_per_frame,
            },
        ));
    }
    Ok(())
}

#[cfg(feature = "alloc")]
fn parse_score_track<P: ScoreParsePolicy, G: ScoreGrammar>(
    mut cursor: Cursor<'_>,
    source_track: u16,
    score: &mut TickScore,
    budget: &mut P,
) -> Result<(), ParseError> {
    let mut builder = ScoreTrackBuilder::new(cursor.remaining());
    let mut running_status = None;
    let mut time = 0_u64;
    let mut saw_end_of_track = false;
    let name = {
        let mut semantics = ScoreTrackSemantics {
            builder: &mut builder,
            score,
            name: TextRange::default(),
        };

        while cursor.remaining() != 0 {
            let delta_offset = cursor.absolute_offset();
            let delta = u64::from(cursor.read_vlq()?);
            time = time
                .checked_add(delta)
                .ok_or(ParseError::at(delta_offset, ParseErrorKind::TickOverflow))?;

            let event_offset = cursor.absolute_offset();
            let first = cursor.read_u8()?;
            let (status, first_data) = if first < 0x80 {
                let status = running_status.ok_or_else(|| {
                    ParseError::at(event_offset, ParseErrorKind::RunningStatusMissing)
                })?;
                (status, Some(first))
            } else {
                if first < 0xf0 {
                    running_status = Some(first);
                }
                (first, None)
            };

            match status {
                0x80..=0xef => {
                    let data = read_score_channel_data(&mut cursor, first_data, status)?;
                    budget.charge_event(event_offset)?;
                    semantics.channel(status, data, time, event_offset, budget, &cursor)?;
                }
                0xff if first_data.is_none() => {
                    let meta_type = cursor.read_u8()?;
                    let length = cursor.read_vlq()?;
                    let payload = cursor.take(
                        usize::try_from(length)
                            .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?,
                    )?;
                    if G::STOP_AT_END_OF_TRACK && meta_type == 0x2f {
                        handle_end_of_track::<P, G>(length, event_offset, &mut cursor, budget)?;
                        saw_end_of_track = true;
                        break;
                    }
                    validate_score_meta(meta_type, length, payload, event_offset)?;
                    budget.charge_event(event_offset)?;
                    semantics.meta(
                        ScoreMetaEvent {
                            meta_type,
                            length,
                            payload,
                            time,
                            event_offset,
                        },
                        budget,
                        &cursor,
                    )?;
                    // SMF SysEx and meta events interrupt channel running status.
                    running_status = None;
                }
                0xf0 | 0xf7 if first_data.is_none() => {
                    let length = usize::try_from(cursor.read_vlq()?)
                        .map_err(|_| cursor.error(ParseErrorKind::SizeOverflow))?;
                    cursor.skip(length)?;
                    budget.charge_event(event_offset)?;
                    running_status = None;
                }
                _ => {
                    return Err(ParseError::at(
                        event_offset,
                        ParseErrorKind::InvalidStatus(status),
                    ));
                }
            }
        }
        semantics.name
    };

    if G::REQUIRE_END_OF_TRACK && !saw_end_of_track {
        return Err(ParseError::at(
            cursor.absolute_offset(),
            ParseErrorKind::MissingEndOfTrack,
        ));
    }
    builder.finish(source_track, name, score, &cursor)
}

#[derive(Clone, Copy)]
struct Cursor<'a> {
    data: &'a [u8],
    position: usize,
    base: usize,
}

impl<'a> Cursor<'a> {
    const fn new(data: &'a [u8], base: usize) -> Self {
        Self {
            data,
            position: 0,
            base,
        }
    }

    fn absolute_offset(self) -> usize {
        self.base + self.position
    }

    fn remaining(self) -> usize {
        self.data.len() - self.position
    }

    fn error(self, kind: ParseErrorKind) -> ParseError {
        ParseError::at(self.absolute_offset(), kind)
    }

    fn read_u8(&mut self) -> Result<u8, ParseError> {
        let byte = self
            .data
            .get(self.position)
            .copied()
            .ok_or_else(|| self.error(ParseErrorKind::UnexpectedEnd))?;
        self.position += 1;
        Ok(byte)
    }

    fn read_u16(&mut self) -> Result<u16, ParseError> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn read_u32(&mut self) -> Result<u32, ParseError> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_tag(&mut self) -> Result<[u8; 4], ParseError> {
        let bytes = self.take(4)?;
        Ok([bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    fn read_vlq(&mut self) -> Result<u32, ParseError> {
        let start = self.absolute_offset();
        let mut value = 0_u32;
        for _ in 0..4 {
            let byte = self.read_u8()?;
            value = (value << 7) | u32::from(byte & 0x7f);
            if byte < 0x80 {
                return Ok(value);
            }
        }
        Err(ParseError::at(
            start,
            ParseErrorKind::VariableLengthQuantityTooLong,
        ))
    }

    fn take(&mut self, size: usize) -> Result<&'a [u8], ParseError> {
        let end = self
            .position
            .checked_add(size)
            .ok_or_else(|| self.error(ParseErrorKind::SizeOverflow))?;
        let slice = self
            .data
            .get(self.position..end)
            .ok_or_else(|| self.error(ParseErrorKind::UnexpectedEnd))?;
        self.position = end;
        Ok(slice)
    }

    fn skip(&mut self, size: usize) -> Result<(), ParseError> {
        self.take(size).map(|_| ())
    }

    fn take_cursor(&mut self, size: usize) -> Result<Self, ParseError> {
        let base = self.absolute_offset();
        let data = self.take(size)?;
        Ok(Self::new(data, base))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    fn one_track(track: &[u8]) -> Vec<u8> {
        let mut file = b"MThd\0\0\0\x06\0\x01\0\x01\x01\xe0MTrk".to_vec();
        file.extend_from_slice(&u32::try_from(track.len()).unwrap().to_be_bytes());
        file.extend_from_slice(track);
        file
    }

    fn tracks(tracks: &[&[u8]]) -> Vec<u8> {
        let mut file = b"MThd\0\0\0\x06\0\x01".to_vec();
        file.extend_from_slice(&u16::try_from(tracks.len()).unwrap().to_be_bytes());
        file.extend_from_slice(&480_u16.to_be_bytes());
        for track in tracks {
            file.extend_from_slice(b"MTrk");
            file.extend_from_slice(&u32::try_from(track.len()).unwrap().to_be_bytes());
            file.extend_from_slice(track);
        }
        file
    }

    fn one_track_with_division(track: &[u8], division: u16) -> Vec<u8> {
        let mut file = one_track(track);
        file[12..14].copy_from_slice(&division.to_be_bytes());
        file
    }

    fn strict_options() -> ScoreParseOptions {
        ScoreParseOptions {
            limits: ScoreParseLimits::DEFAULT,
            mode: ScoreParseMode::Strict,
        }
    }

    #[test]
    fn packed_control_change_is_ten_bytes_and_keeps_value_getters() {
        assert_eq!(core::mem::size_of::<TickControlChange>(), 10);
        let control = TickControlChange::new(123, 64, 127);
        assert_eq!(control.time(), 123);
        assert_eq!(control.number(), 64);
        assert_eq!(control.value(), 127);
        assert_eq!(control, TickControlChange::new(123, 64, 127));
    }

    fn track_notes(score: &TickScore, track: usize) -> Vec<TickNote> {
        score.track_notes(track).unwrap().iter().collect()
    }

    fn long_duration_note(final_delta: u8) -> Vec<u8> {
        let mut track = vec![0, 0x90, 60, 100];
        for _ in 0..16 {
            track.extend_from_slice(&[0xff, 0xff, 0xff, 0x7f, 0xff, 0x01, 0x00]);
        }
        track.extend_from_slice(&[final_delta, 0x80, 60, 0, 0, 0xff, 0x2f, 0]);
        track
    }

    #[test]
    fn scans_channel_running_status_and_meta_events() {
        let file = one_track(&[
            0x00, 0x90, 60, 100, // note on
            0x81, 0x00, 60, 0, // delta 128, running-status note off
            0x00, 0xff, 0x2f, 0x00, // end of track
        ]);

        let summary = scan_smf(&file).unwrap();
        assert_eq!(summary.header.format, Format::Parallel);
        assert_eq!(summary.header.track_count, 1);
        assert_eq!(summary.events, 3);
        assert_eq!(summary.channel_events, 2);
        assert_eq!(summary.meta_events, 1);
        assert_eq!(summary.max_delta_ticks, 128);
        assert_eq!(summary.bytes_consumed, file.len());
        assert_eq!(summary.trailing_bytes, 0);
    }

    #[test]
    fn scans_sysex_and_one_byte_channel_messages() {
        let file = one_track(&[
            0x00, 0xc2, 10, // program change
            0x00, 11, // running-status program change
            0x00, 0xf0, 0x03, 1, 2, 0xf7, // sysex payload
            0x00, 0xff, 0x2f, 0x00,
        ]);
        let summary = scan_smf(&file).unwrap();
        assert_eq!(summary.events, 4);
        assert_eq!(summary.channel_events, 2);
        assert_eq!(summary.sysex_events, 1);
    }

    #[test]
    fn reports_missing_running_status_at_absolute_offset() {
        let file = one_track(&[0x00, 60, 100]);
        let error = scan_smf(&file).unwrap_err();
        assert_eq!(error.offset, 23);
        assert_eq!(error.kind, ParseErrorKind::RunningStatusMissing);
        assert_eq!(parse_score_smf(&file).unwrap_err(), error);
    }

    #[test]
    fn score_channel_fast_path_preserves_invalid_data_offsets() {
        for (track, offset) in [
            (&[0x00, 0x90, 0x80, 1][..], 24),  // explicit first data byte
            (&[0x00, 0x90, 60, 0x80][..], 25), // explicit second data byte
            (&[0x00, 0x90, 60, 1, 0x00, 60, 0x80][..], 28), // running-status second data byte
            (&[0x00, 0xc0, 0x80][..], 24),     // one-byte program change
        ] {
            let error = parse_score_smf(&one_track(track)).unwrap_err();
            assert_eq!(error.offset, offset);
            assert_eq!(error.kind, ParseErrorKind::InvalidDataByte(0x80));
        }
    }

    fn assert_all_decoders_reject_running_status_after(
        interruption: &[u8],
        expected_offset: usize,
    ) {
        let mut track = vec![0x00, 0x90, 60, 1];
        track.extend_from_slice(interruption);
        track.extend_from_slice(&[0x01, 60, 0, 0x00, 0xff, 0x2f, 0x00]);
        let file = one_track(&track);
        let error = ParseError::at(expected_offset, ParseErrorKind::RunningStatusMissing);
        assert_eq!(scan_smf(&file).unwrap_err(), error);
        assert_eq!(parse_smf(&file).unwrap_err(), error);
        assert_eq!(parse_score_smf(&file).unwrap_err(), error);
    }

    #[test]
    fn sysex_and_meta_events_cancel_running_status_in_every_decoder() {
        // Track-local offset 9 is absolute offset 31 after the 22-byte SMF
        // header/MTrk framing. F0 and F7 have the same encoded size here.
        assert_all_decoders_reject_running_status_after(&[0x00, 0xf0, 0x01, 0x7f], 31);
        assert_all_decoders_reject_running_status_after(&[0x00, 0xf7, 0x01, 0x7f], 31);
        // FF + type + length + one payload byte moves the next data byte one
        // byte later than the SysEx fixtures.
        assert_all_decoders_reject_running_status_after(&[0x00, 0xff, 0x01, 0x01, b'x'], 32);
    }

    fn score_limits() -> ScoreParseLimits {
        ScoreParseLimits {
            max_input_bytes: 1_024,
            max_source_tracks: 16,
            max_track_bytes: 1_024,
            max_events: 64,
            max_note_starts: 64,
            max_text_bytes: 1_024,
        }
    }

    fn assert_score_limit(
        result: Result<TickScore, ParseError>,
        offset: usize,
        resource: ScoreResource,
        limit: usize,
    ) {
        assert_eq!(
            result.unwrap_err(),
            ParseError::at(
                offset,
                ParseErrorKind::ResourceLimitExceeded { resource, limit }
            )
        );
    }

    #[test]
    fn limited_score_parser_enforces_input_track_and_event_limits() {
        let file = one_track(&[0x00, 0xff, 0x2f, 0x00]);

        let mut limits = score_limits();
        limits.max_input_bytes = file.len() - 1;
        assert_score_limit(
            parse_score_smf_with_limits(&file, limits),
            0,
            ScoreResource::InputBytes,
            limits.max_input_bytes,
        );

        let two_tracks = tracks(&[&[0x00, 0xff, 0x2f, 0x00], &[0x00, 0xff, 0x2f, 0x00]]);
        let mut limits = score_limits();
        limits.max_source_tracks = 1;
        assert_score_limit(
            parse_score_smf_with_limits(&two_tracks, limits),
            10,
            ScoreResource::SourceTracks,
            1,
        );

        let mut limits = score_limits();
        limits.max_track_bytes = 3;
        assert_score_limit(
            parse_score_smf_with_limits(&file, limits),
            18,
            ScoreResource::TrackBytes,
            3,
        );

        let two_events = one_track(&[0x00, 0x90, 60, 1, 0x00, 0xff, 0x2f, 0x00]);
        let mut limits = score_limits();
        limits.max_events = 1;
        assert_score_limit(
            parse_score_smf_with_limits(&two_events, limits),
            27,
            ScoreResource::Events,
            1,
        );
    }

    #[test]
    fn limited_score_parser_charges_dangling_and_overlapping_note_starts() {
        let dangling = one_track(&[0x00, 0x90, 60, 1, 0x00, 0xff, 0x2f, 0x00]);
        let mut limits = score_limits();
        limits.max_note_starts = 0;
        assert_score_limit(
            parse_score_smf_with_limits(&dangling, limits),
            23,
            ScoreResource::NoteStarts,
            0,
        );

        let overlap = one_track(&[
            0x00, 0x90, 60, 1, // primary note start
            0x00, 60, 2, // overlapping running-status note start
            0x00, 0xff, 0x2f, 0x00,
        ]);
        let mut limits = score_limits();
        limits.max_note_starts = 1;
        assert_score_limit(
            parse_score_smf_with_limits(&overlap, limits),
            27,
            ScoreResource::NoteStarts,
            1,
        );
    }

    #[test]
    fn limited_score_parser_counts_normalized_invalid_utf8_text() {
        let file = one_track(&[
            0x00, 0xff, 0x03, 0x02, b'a', 0xff, // normalized to a + U+FFFD (4 bytes)
            0x00, 0x90, 60, 1, 0x01, 0x80, 60, 0, 0x00, 0xff, 0x2f, 0x00,
        ]);
        let mut limits = score_limits();
        limits.max_text_bytes = 3;
        assert_score_limit(
            parse_score_smf_with_limits(&file, limits),
            23,
            ScoreResource::TextBytes,
            3,
        );

        limits.max_text_bytes = 4;
        let score = parse_score_smf_with_limits(&file, limits).unwrap();
        assert_eq!(
            score.text(score.tracks()[0].name()),
            Some("a\u{fffd}".as_bytes())
        );
    }

    #[test]
    fn unlimited_score_paths_have_identical_semantics() {
        let file = one_track(&[
            0x00, 0xff, 0x03, 0x01, b'n', 0x00, 0x90, 60, 1, 0x01, 60, 0, 0x00, 0xff, 0x51, 0x03,
            0x07, 0xa1, 0x20, 0x00, 0xff, 0x2f, 0x00,
        ]);
        assert_eq!(
            parse_score_smf(&file).unwrap(),
            parse_score_smf_with_limits(&file, ScoreParseLimits::UNLIMITED).unwrap()
        );
    }

    #[test]
    fn score_parse_defaults_are_finite_and_compatible() {
        assert_eq!(
            ScoreParseLimits::default(),
            ScoreParseLimits {
                max_input_bytes: 64 * 1024 * 1024,
                max_source_tracks: 4_096,
                max_track_bytes: 16 * 1024 * 1024,
                max_events: 2_000_000,
                max_note_starts: 1_000_000,
                max_text_bytes: 16 * 1024 * 1024,
            }
        );
        assert_eq!(ScoreParseLimits::DEFAULT, ScoreParseLimits::default());
        assert_eq!(
            ScoreParseOptions::default(),
            ScoreParseOptions {
                limits: ScoreParseLimits::DEFAULT,
                mode: ScoreParseMode::Compatible,
            }
        );
    }

    #[test]
    fn options_compatible_ignores_post_eot_but_legacy_limits_do_not() {
        let file = one_track(&[
            0x00, 0xff, 0x2f, 0x00, // EOT
            0x00, 0x90, 60, 1, 0x01, 0x80, 60, 0, // valid note after EOT
        ]);
        let compatible = parse_score_smf_with_options(&file, ScoreParseOptions::default()).unwrap();
        assert_eq!(compatible.note_count(), 0);

        // The legacy limited entry point intentionally retains the old
        // permissive grammar and therefore still decodes post-EOT events.
        let legacy = parse_score_smf_with_limits(&file, ScoreParseLimits::DEFAULT).unwrap();
        assert_eq!(legacy.note_count(), 1);

        assert_eq!(
            parse_score_smf_with_options(&file, strict_options()).unwrap_err(),
            ParseError::at(26, ParseErrorKind::EventAfterEndOfTrack)
        );
    }

    #[test]
    fn strict_options_validate_eot_and_track_boundaries() {
        let invalid_payload = one_track(&[0x00, 0xff, 0x2f, 0x01, 0]);
        assert_eq!(
            parse_score_smf_with_options(&invalid_payload, strict_options()).unwrap_err(),
            ParseError::at(23, ParseErrorKind::InvalidEndOfTrackLength(1))
        );
        // Compatible mode treats the first EOT as a stop marker even when its
        // payload is not standards-conforming.
        assert!(
            parse_score_smf_with_options(&invalid_payload, ScoreParseOptions::default()).is_ok()
        );

        let duplicate = one_track(&[
            0x00, 0xff, 0x2f, 0x00, // first EOT
            0x00, 0xff, 0x2f, 0x00, // second event begins at absolute byte 26
        ]);
        assert_eq!(
            parse_score_smf_with_options(&duplicate, strict_options()).unwrap_err(),
            ParseError::at(26, ParseErrorKind::EventAfterEndOfTrack)
        );

        let missing = one_track(&[0x00, 0x90, 60, 1]);
        assert_eq!(
            parse_score_smf_with_options(&missing, strict_options()).unwrap_err(),
            ParseError::at(26, ParseErrorKind::MissingEndOfTrack)
        );
        assert!(parse_score_smf_with_options(&missing, ScoreParseOptions::default()).is_ok());
    }

    #[test]
    fn strict_options_validate_trailing_and_division_encodings() {
        let mut trailing = one_track(&[0x00, 0xff, 0x2f, 0x00]);
        trailing.extend_from_slice(b"tail");
        assert_eq!(
            parse_score_smf_with_options(&trailing, strict_options()).unwrap_err(),
            ParseError::at(26, ParseErrorKind::TrailingBytes)
        );
        assert_eq!(
            parse_score_smf_with_options(&trailing, ScoreParseOptions::default())
                .unwrap()
                .trailing_bytes(),
            4
        );

        let eot = [0x00, 0xff, 0x2f, 0x00];
        let tpq_zero = one_track_with_division(&eot, 0);
        assert_eq!(
            parse_score_smf_with_options(&tpq_zero, strict_options()).unwrap_err(),
            ParseError::at(12, ParseErrorKind::InvalidTicksPerQuarter)
        );

        let valid_smpte = one_track_with_division(&eot, 0xe728); // -25 fps, 40 ticks/frame
        assert_eq!(
            parse_score_smf_with_options(&valid_smpte, strict_options())
                .unwrap()
                .header()
                .division,
            0xe728
        );
        let invalid_frame = one_track_with_division(&eot, 0xe628);
        assert_eq!(
            parse_score_smf_with_options(&invalid_frame, strict_options()).unwrap_err(),
            ParseError::at(
                12,
                ParseErrorKind::InvalidSmpteDivision {
                    frames_per_second: -26,
                    ticks_per_frame: 40,
                }
            )
        );
        let zero_ticks = one_track_with_division(&eot, 0xe800);
        assert_eq!(
            parse_score_smf_with_options(&zero_ticks, strict_options()).unwrap_err(),
            ParseError::at(
                12,
                ParseErrorKind::InvalidSmpteDivision {
                    frames_per_second: -24,
                    ticks_per_frame: 0,
                }
            )
        );
    }

    #[test]
    fn strict_options_preserve_resource_error_precedence() {
        let mut zero_events = strict_options();
        zero_events.limits.max_events = 0;
        let invalid_payload = one_track(&[0x00, 0xff, 0x2f, 0x01, 0]);
        // Structural EOT validation wins before charging its event budget.
        assert_eq!(
            parse_score_smf_with_options(&invalid_payload, zero_events).unwrap_err(),
            ParseError::at(23, ParseErrorKind::InvalidEndOfTrackLength(1))
        );

        let valid_eot = one_track(&[0x00, 0xff, 0x2f, 0x00]);
        assert_score_limit(
            parse_score_smf_with_options(&valid_eot, zero_events),
            23,
            ScoreResource::Events,
            0,
        );

        let mut one_event = strict_options();
        one_event.limits.max_events = 1;
        let after_eot = one_track(&[0x00, 0xff, 0x2f, 0x00, 0x00, 0xff, 0x2f, 0x00]);
        // EOT framing wins before the second physical event would be charged.
        assert_eq!(
            parse_score_smf_with_options(&after_eot, one_event).unwrap_err(),
            ParseError::at(26, ParseErrorKind::EventAfterEndOfTrack)
        );
    }

    #[test]
    fn strict_and_compatible_options_match_legacy_on_well_framed_scores() {
        let file = one_track(&[0x00, 0x90, 60, 1, 0x01, 0x80, 60, 0, 0x00, 0xff, 0x2f, 0x00]);
        let legacy = parse_score_smf(&file).unwrap();
        assert_eq!(
            parse_score_smf_with_options(&file, ScoreParseOptions::default()).unwrap(),
            legacy
        );
        assert_eq!(
            parse_score_smf_with_options(&file, strict_options()).unwrap(),
            legacy
        );
    }

    #[test]
    fn rejects_five_byte_vlq() {
        let file = one_track(&[0x80, 0x80, 0x80, 0x80, 0x00, 0xff, 0x2f, 0]);
        let error = scan_smf(&file).unwrap_err();
        assert_eq!(error.offset, 22);
        assert_eq!(error.kind, ParseErrorKind::VariableLengthQuantityTooLong);
    }

    #[test]
    fn vlq_fast_path_preserves_one_to_four_byte_values_and_offsets() {
        for (bytes, expected) in [
            (&[0x7f][..], 0x7f),
            (&[0x81, 0x00][..], 0x80),
            (&[0xc0, 0x80, 0x00][..], 0x10_0000),
            (&[0xff, 0xff, 0xff, 0x7f][..], 0x0fff_ffff),
        ] {
            let mut cursor = Cursor::new(bytes, 37);
            assert_eq!(cursor.read_vlq().unwrap(), expected);
            assert_eq!(cursor.absolute_offset(), 37 + bytes.len());
        }

        let mut too_long = Cursor::new(&[0x80, 0x80, 0x80, 0x80, 0][..], 37);
        assert_eq!(
            too_long.read_vlq().unwrap_err(),
            ParseError::at(37, ParseErrorKind::VariableLengthQuantityTooLong)
        );
        // The fifth byte remains unread, exactly as the generic loop did.
        assert_eq!(too_long.absolute_offset(), 41);
    }

    #[test]
    fn reports_trailing_bytes_without_rejecting_them() {
        let mut file = one_track(&[0x00, 0xff, 0x2f, 0]);
        file.extend_from_slice(b"junk");
        let summary = scan_smf(&file).unwrap();
        assert_eq!(summary.trailing_bytes, 4);
        assert_eq!(summary.bytes_consumed + summary.trailing_bytes, file.len());
    }

    #[test]
    fn owned_arena_keeps_compact_headers_and_shared_payloads() {
        let file = one_track(&[
            0x00, 0x90, 60, 100, 0x00, 60, 0, 0x00, 0xff, 0x01, 0x03, b'a', b'b', b'c', 0x00, 0xf0,
            0x03, 1, 2, 0xf7, 0x00, 0xff, 0x2f, 0x00,
        ]);
        let owned = parse_smf(&file).unwrap();

        assert_eq!(core::mem::size_of::<EventRecord>(), 16);
        assert_eq!(owned.tracks(), &[TrackRange { start: 0, len: 5 }]);
        assert_eq!(owned.events().len(), 5);
        assert_eq!(owned.event_data(0), Some([60, 100].as_slice()));
        assert_eq!(owned.event_data(1), Some([60, 0].as_slice()));
        assert_eq!(owned.events()[2].meta_type(), Some(0x01));
        assert_eq!(owned.event_data(2), Some(b"abc".as_slice()));
        assert_eq!(owned.event_data(3), Some([1, 2, 0xf7].as_slice()));
        assert_eq!(owned.track_events(0).unwrap().len(), 5);
        assert_eq!(owned.heap_bytes(), 8 + 5 * 16 + 6);
    }

    #[test]
    fn score_parser_covers_tick_event_categories() {
        let file = one_track(&[
            0x00, 0xff, 0x03, 0x04, b'b', b'a', b'n', b'd', // source name
            0x00, 0xc1, 5, // program
            0x00, 0xb1, 7, 99, // control
            0x00, 0xe1, 0, 64, // centered pitch bend
            0x00, 0xb1, 64, 127, // pedal on
            0x00, 0x91, 60, 100, // note on
            0x0a, 0x81, 60, 0, // note off
            0x00, 0xb1, 64, 0, // pedal off
            0x00, 0xff, 0x05, 0x02, b'l', b'a', // lyric: metadata channel 0
            0x00, 0xff, 0x51, 0x03, 0x07, 0xa1, 0x20, // 500000 mspq
            0x00, 0xff, 0x58, 0x04, 3, 2, 24, 8, // 3/4
            0x00, 0xff, 0x59, 0x02, 0xfb, 1, // -5 minor
            0x00, 0xff, 0x06, 0x01, b'A', // marker
            0x00, 0xff, 0x2f, 0x00,
        ]);
        let score = parse_score_smf(&file).unwrap();

        assert_eq!(score.ticks_per_quarter(), 480);
        assert_eq!(score.tracks().len(), 2);
        // Ordered by channel/program: lyric channel 0 group precedes channel 1.
        assert_eq!(score.tracks()[0].channel(), 0);
        assert_eq!(score.tracks()[0].program(), 0);
        assert_eq!(score.tracks()[1].channel(), 1);
        assert_eq!(score.tracks()[1].program(), 5);
        assert_eq!(
            score.text(score.tracks()[0].name()),
            Some(b"band".as_slice())
        );
        assert_eq!(score.track_lyrics(0).unwrap()[0].time(), 10);
        assert_eq!(
            score.text(score.track_lyrics(0).unwrap()[0].text()),
            Some(b"la".as_slice())
        );
        assert_eq!(
            track_notes(&score, 1),
            vec![TickNote {
                time: 0,
                duration: 10,
                pitch: 60,
                velocity: 100
            }]
        );
        assert_eq!(score.track_controls(1).unwrap()[0].number(), 7);
        assert_eq!(score.track_pitch_bends(1).unwrap()[0].value(), 0);
        assert_eq!(
            score.track_pedals(1).unwrap(),
            &[TickPedal {
                time: 0,
                duration: 10
            }]
        );
        assert_eq!(score.tempos()[0].microseconds_per_quarter(), 500_000);
        assert_eq!(score.time_signatures()[0].denominator(), 4);
        assert_eq!(score.key_signatures()[0].key(), -5);
        assert_eq!(score.key_signatures()[0].tonality(), 1);
        assert_eq!(score.text(score.markers()[0].text()), Some(b"A".as_slice()));
    }

    #[test]
    fn score_parser_uses_running_status_program_order_and_channel_nine_drums() {
        let file = one_track(&[
            0x00, 0xc2, 10, // create program 10 first
            0x00, 0x92, 60, 100, 0x01, 60, 0, // running-status note on velocity zero
            0x00, 0xc2, 1, 0x00, 0x92, 61, 90, 0x01, 61, 0, // running status closes 61
            0x00, 0x99, 36, 127, 0x01, 36, 0, 0x00, 0xff, 0x2f, 0x00,
        ]);
        let score = parse_score_smf(&file).unwrap();
        assert_eq!(score.tracks().len(), 3);
        assert_eq!(
            score
                .tracks()
                .iter()
                .map(|track| (track.channel(), track.program(), track.is_drum()))
                .collect::<Vec<_>>(),
            vec![(2, 1, false), (2, 10, false), (9, 0, true)]
        );
        assert_eq!(track_notes(&score, 0)[0].pitch(), 61);
        assert_eq!(track_notes(&score, 1)[0].pitch(), 60);
    }

    #[test]
    fn score_parser_pairs_overlapping_notes_fifo_and_discards_orphans_and_dangling_notes() {
        let file = one_track(&[
            0x00, 0x90, 60, 10, // first note on at 0
            0x01, 60, 20, // second note on at 1
            0x01, 0x80, 60, 0, // first closes at 2
            0x01, 60, 0, // second closes at 3
            0x00, 0x80, 61, 0, // orphan note off
            0x00, 0x90, 62, 30, // dangling note on
            0x00, 0xff, 0x2f, 0x00,
        ]);
        let score = parse_score_smf(&file).unwrap();
        let notes = track_notes(&score, 0);
        assert_eq!(notes.len(), 2);
        assert_eq!(
            notes[0],
            TickNote {
                time: 0,
                duration: 2,
                pitch: 60,
                velocity: 10
            }
        );
        assert_eq!(
            notes[1],
            TickNote {
                time: 1,
                duration: 2,
                pitch: 60,
                velocity: 20
            }
        );
    }

    #[test]
    fn score_parser_handles_zero_and_one_open_note_cases() {
        let only_orphan = one_track(&[0x00, 0x80, 60, 0, 0x00, 0xff, 0x2f, 0]);
        let empty = parse_score_smf(&only_orphan).unwrap();
        assert_eq!(empty.tracks().len(), 0);
        assert_eq!(empty.notes().len(), 0);

        let one_note = one_track(&[0x00, 0x90, 60, 100, 0x05, 0x80, 60, 0, 0x00, 0xff, 0x2f, 0]);
        let score = parse_score_smf(&one_note).unwrap();
        assert_eq!(
            track_notes(&score, 0),
            vec![TickNote {
                time: 0,
                duration: 5,
                pitch: 60,
                velocity: 100
            }]
        );
    }

    #[test]
    fn note_capacity_hint_is_claimed_once_by_the_first_note_group() {
        const HINT: usize = 2_048;
        let cursor = Cursor::new(&[], 0);

        let meta_only = ScoreTrackBuilder::new(HINT * 6);
        assert_eq!(meta_only.note_capacity_hint, Some(HINT));
        assert!(meta_only.groups.is_empty());

        let mut builder = ScoreTrackBuilder::new(HINT * 6);
        builder.add_lyric(0, TextRange::default(), &cursor).unwrap();
        assert_eq!(builder.note_capacity_hint, Some(HINT));
        assert_eq!(builder.groups[0].notes.capacity(), 0);

        builder.add_note(1, 60, 100, 0, &cursor).unwrap();
        assert_eq!(builder.note_capacity_hint, None);
        assert!(builder.groups[1].notes.capacity() >= HINT);

        builder.add_note(2, 61, 100, 0, &cursor).unwrap();
        assert!(builder.groups[2].notes.capacity() < HINT);
    }

    #[test]
    fn packed_open_note_state_preserves_index_and_u32_start_boundaries() {
        let common = ScoreTrackBuilder::pack_open_note_state(123, u64::from(u32::MAX - 1));
        assert_eq!(ScoreTrackBuilder::open_note_index(common), 123);
        assert_eq!(
            ScoreTrackBuilder::open_note_start(common),
            Some(u64::from(u32::MAX - 1))
        );

        for time in [u64::from(u32::MAX), u64::from(u32::MAX) + 1] {
            let fallback = ScoreTrackBuilder::pack_open_note_state(u32::MAX - 1, time);
            assert_eq!(ScoreTrackBuilder::open_note_index(fallback), u32::MAX - 1);
            assert_eq!(ScoreTrackBuilder::open_note_start(fallback), None);
        }
    }

    #[test]
    fn note_segments_narrow_at_u32_boundary_and_promote_losslessly() {
        assert_eq!(core::mem::size_of::<NarrowNoteRow>(), 10);
        assert_eq!(core::mem::size_of::<WideNoteRow>(), 18);
        let narrow_row = NarrowNoteRow::new(1, 2, 60, 100).with_duration(3);
        assert_eq!(narrow_row.time(), 1);
        assert_eq!(narrow_row.duration(), 3);
        assert_eq!(narrow_row.note().pitch(), 60);
        let wide_row = WideNoteRow::new(4, 5, 61, 101).with_duration(6);
        assert_eq!(wide_row.time(), 4);
        assert_eq!(wide_row.duration(), 6);
        assert_eq!(wide_row.note().velocity(), 101);

        let cursor = Cursor::new(&[], 0);
        let mut narrow_builder = NoteColumnsBuilder::default();
        let narrow_index = narrow_builder
            .push(u64::from(u32::MAX), 60, 100, &cursor)
            .unwrap();
        narrow_builder.complete(narrow_index, u64::from(u32::MAX));
        let narrow = narrow_builder.into_segment();
        assert!(matches!(&narrow.timing, NoteTimingColumns::Narrow(_)));
        assert_eq!(
            TickNoteView { segment: &narrow }.iter().collect::<Vec<_>>(),
            vec![TickNote {
                time: u64::from(u32::MAX),
                duration: u64::from(u32::MAX),
                pitch: 60,
                velocity: 100,
            }]
        );

        let wide_value = u64::from(u32::MAX) + 1;
        let mut wide_builder = NoteColumnsBuilder::default();
        let first = wide_builder.push(0, 61, 101, &cursor).unwrap();
        wide_builder.complete(first, 0);
        let second = wide_builder.push(wide_value, 62, 102, &cursor).unwrap();
        wide_builder.complete(second, u64::MAX);
        let wide = wide_builder.into_segment();
        assert!(matches!(&wide.timing, NoteTimingColumns::Wide(_)));
        assert_eq!(
            TickNoteView { segment: &wide }.iter().collect::<Vec<_>>(),
            vec![
                TickNote {
                    time: 0,
                    duration: 0,
                    pitch: 61,
                    velocity: 101,
                },
                TickNote {
                    time: wide_value,
                    duration: u64::MAX,
                    pitch: 62,
                    velocity: 102,
                },
            ]
        );
    }

    #[test]
    fn parser_promotes_a_segment_only_after_crossing_the_u32_tick_boundary() {
        let boundary = parse_score_smf(&one_track(&long_duration_note(15))).unwrap();
        assert!(matches!(
            &boundary.note_segments[0].timing,
            NoteTimingColumns::Narrow(_)
        ));
        assert_eq!(track_notes(&boundary, 0)[0].duration(), u64::from(u32::MAX));

        let wide = parse_score_smf(&one_track(&long_duration_note(16))).unwrap();
        assert!(matches!(
            &wide.note_segments[0].timing,
            NoteTimingColumns::Wide(_)
        ));
        assert_eq!(track_notes(&wide, 0)[0].duration(), u64::from(u32::MAX) + 1);
    }

    #[test]
    fn score_parser_promotes_three_overlaps_fifo_across_program_changes() {
        let file = one_track(&[
            0x00, 0xc0, 1, // program one: first two starts
            0x00, 0x90, 60, 10, 0x01, 60, 20, 0x00, 0xc0, 2, // program two: third start
            0x01, 0x90, 60, 30, 0x01, 0x80, 60, 0, // close first at tick 3
            0x01, 60, 0, // then second at tick 4
            0x01, 60, 0, // then third at tick 5
            0x00, 0xff, 0x2f, 0,
        ]);
        let score = parse_score_smf(&file).unwrap();
        assert_eq!(
            score
                .tracks()
                .iter()
                .map(|track| track.program())
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            track_notes(&score, 0),
            vec![
                TickNote {
                    time: 0,
                    duration: 3,
                    pitch: 60,
                    velocity: 10
                },
                TickNote {
                    time: 1,
                    duration: 3,
                    pitch: 60,
                    velocity: 20
                },
            ]
        );
        assert_eq!(
            track_notes(&score, 1),
            vec![TickNote {
                time: 2,
                duration: 3,
                pitch: 60,
                velocity: 30
            }]
        );
        assert_eq!(
            score
                .notes()
                .iter()
                .map(|note| note.velocity())
                .collect::<Vec<_>>(),
            vec![10, 20, 30]
        );
    }

    #[test]
    fn score_equality_ignores_the_lazy_global_note_cache() {
        let file = one_track(&[0x00, 0x90, 60, 100, 0x01, 0x80, 60, 0, 0x00, 0xff, 0x2f, 0]);
        let flattened = parse_score_smf(&file).unwrap();
        let unflattened = parse_score_smf(&file).unwrap();
        assert_eq!(flattened.notes().len(), 1);
        assert_eq!(flattened, unflattened);
    }

    #[test]
    fn score_note_count_and_heap_bytes_do_not_flatten_note_segments() {
        let file = one_track(&[0x00, 0x90, 60, 100, 0x01, 0x80, 60, 0, 0x00, 0xff, 0x2f, 0]);
        let score = parse_score_smf(&file).unwrap();
        let heap_before = score.heap_bytes();
        assert!(score.notes.get().is_none());
        assert_eq!(score.note_count(), 1);
        assert_eq!(score.heap_bytes(), heap_before);
        assert!(score.notes.get().is_none());
        assert_eq!(score.track_notes(0).unwrap().iter().count(), 1);
        assert_eq!(score.heap_bytes(), heap_before);
        assert!(score.notes.get().is_none());
    }

    #[test]
    fn score_parser_stably_sorts_global_events_across_source_tracks() {
        let first = [
            0x0a, 0xff, 0x51, 0x03, 0x07, 0xa1, 0x20, 0x00, 0xff, 0x06, 0x01, b'x', 0x00, 0xff,
            0x2f, 0x00,
        ];
        let second = [
            0x05, 0xff, 0x51, 0x03, 0x06, 0x1a, 0x80, 0x05, 0xff, 0x51, 0x03, 0x05, 0x16, 0x15,
            0x00, 0xff, 0x06, 0x01, b'y', 0x00, 0xff, 0x2f, 0x00,
        ];
        let score = parse_score_smf(&tracks(&[&first, &second])).unwrap();
        assert_eq!(
            score
                .tempos()
                .iter()
                .map(|tempo| tempo.time())
                .collect::<Vec<_>>(),
            vec![5, 10, 10]
        );
        // Equal-time global events retain source parsing order (tempo from
        // the first source track before the latter track's equal-time tempo).
        assert_eq!(
            score
                .tempos()
                .iter()
                .map(|tempo| tempo.microseconds_per_quarter())
                .collect::<Vec<_>>(),
            vec![400_000, 500_000, 333_333]
        );
        assert_eq!(
            score
                .markers()
                .iter()
                .map(|marker| score.text(marker.text()).unwrap())
                .collect::<Vec<_>>(),
            vec![b"x".as_slice(), b"y".as_slice()]
        );
    }

    #[test]
    fn score_parser_rejects_malformed_recognised_meta_events() {
        let file = one_track(&[0x00, 0xff, 0x51, 0x02, 0, 0, 0x00, 0xff, 0x2f, 0]);
        let error = parse_score_smf(&file).unwrap_err();
        assert_eq!(error.offset, 23);
        assert_eq!(
            error.kind,
            ParseErrorKind::InvalidMetaEvent {
                meta_type: 0x51,
                length: 2
            }
        );
    }

    #[test]
    fn score_parser_replaces_invalid_utf8_text() {
        let file = one_track(&[
            0x00, 0xff, 0x03, 0x02, b'a', 0xff, // invalid track name byte
            0x00, 0x90, 60, 1, 0x01, 0x80, 60, 0, 0x00, 0xff, 0x2f, 0,
        ]);
        let score = parse_score_smf(&file).unwrap();
        assert_eq!(
            score.text(score.tracks()[0].name()),
            Some("a\u{fffd}".as_bytes())
        );
    }

    #[test]
    fn generated_style_fixture_has_compact_score_semantics_and_counts() {
        let file = one_track(&[
            0x00, 0xff, 0x03, 0x03, b'g', b'e', b'n', 0x00, 0xc0, 4, 0x00, 0x90, 64, 100, 0x81,
            0x70, 0x80, 64, 0, // 240 tick note
            0x00, 0x90, 67, 90, 0x81, 0x70, 0x80, 67, 0, 0x00, 0xff, 0x2f, 0x00,
        ]);
        let score = parse_score_smf(&file).unwrap();
        assert_eq!(score.tracks().len(), 1);
        assert_eq!(score.notes().len(), 2);
        assert_eq!(track_notes(&score, 0)[1].time(), 240);
        assert_eq!(track_notes(&score, 0)[1].duration(), 240);
        assert_eq!(
            score.text(score.tracks()[0].name()),
            Some(b"gen".as_slice())
        );
        assert!(score.heap_bytes() >= core::mem::size_of_val(score.notes()));
    }
}
