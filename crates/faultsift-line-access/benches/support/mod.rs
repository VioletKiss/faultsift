use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
#[cfg(windows)]
use std::path::Component;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[cfg(windows)]
use faultsift_file_access::MappingFallbackReason;
use faultsift_file_access::{
    BackendKind, ByteLength, FileAccessDiagnostics, FileAccessOptions, FileSnapshot,
};
use faultsift_line_access::{
    BuildControl, LineAccessError, LineDescriptor, LineIndex, LineIndexOptions, LineNumber,
    LineRange, LineSpan, LineTerminator, PhysicalLineCursor, ScanOptions,
};

pub const KIB: u64 = 1_024;
pub const MIB: u64 = 1_024 * KIB;
pub const RANDOM_SEED: u64 = 0x4653_3030_395f_4c41;
pub const CRITERION_VERSION: &str = "0.7.0";
pub const FIXTURE_WRITE_BUFFER_BYTES: usize = 64 * KIB as usize;
pub const CHECKPOINT_BUDGET_CANDIDATES: [u64; 3] = [16 * MIB, 32 * MIB, 64 * MIB];
pub const SCAN_CHUNK_CANDIDATES: [u64; 2] = [64 * KIB, MIB];
pub const BENCHMARK_CONCURRENCY_LEVELS: [usize; 2] = [1, 4];
pub const BASELINE_CHECKPOINT_BUDGET_BYTES: u64 = 32 * MIB;
pub const BASELINE_SCAN_CHUNK_BYTES: u64 = MIB;
pub const PATHOLOGICAL_CHECKPOINT_BUDGET_BYTES: u64 = 3 * 8;

const MAX_REPRESENTATIVE_FIXTURE_MIB: u64 = 512;
const MAX_HUGE_LINE_MIB: u64 = 256;
const MAX_LOOKUP_REQUESTS: usize = 16_384;
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminatorStyle {
    Lf,
    CrLf,
}

impl TerminatorStyle {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Lf => "lf",
            Self::CrLf => "crlf",
        }
    }

    pub const fn bytes(self) -> &'static [u8] {
        match self {
            Self::Lf => b"\n",
            Self::CrLf => b"\r\n",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FixtureShape {
    Representative {
        average_physical_bytes: u64,
        terminator: TerminatorStyle,
    },
    NewlineDense,
    HugeLine {
        content_bytes: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureSpec {
    pub name: String,
    pub target_bytes: u64,
    pub seed: u64,
    pub shape: FixtureShape,
}

impl FixtureSpec {
    pub fn representative(
        average_physical_bytes: u64,
        terminator: TerminatorStyle,
        target_bytes: u64,
        seed: u64,
    ) -> Self {
        assert!(average_physical_bytes > terminator.bytes().len() as u64 + 8);
        Self {
            name: format!("{}-avg-{average_physical_bytes}", terminator.label()),
            target_bytes,
            seed,
            shape: FixtureShape::Representative {
                average_physical_bytes,
                terminator,
            },
        }
    }

    pub fn newline_dense(target_bytes: u64, seed: u64) -> Self {
        Self {
            name: "newline-dense-lf".to_owned(),
            target_bytes,
            seed,
            shape: FixtureShape::NewlineDense,
        }
    }

    pub fn huge_line(content_bytes: u64, seed: u64) -> Self {
        Self {
            name: format!("huge-line-{content_bytes}-bytes"),
            target_bytes: content_bytes + 1,
            seed,
            shape: FixtureShape::HugeLine { content_bytes },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureMetadata {
    pub name: String,
    pub seed: u64,
    pub length: u64,
    pub line_count: u64,
    pub content_bytes: u64,
    pub lf_lines: u64,
    pub crlf_lines: u64,
    pub physical_checksum: u64,
    pub content_checksum: u64,
}

impl FixtureMetadata {
    pub fn average_physical_bytes(&self) -> f64 {
        if self.line_count == 0 {
            0.0
        } else {
            self.length as f64 / self.line_count as f64
        }
    }
}

#[derive(Debug)]
pub struct FixtureFile {
    path: PathBuf,
    metadata: FixtureMetadata,
}

impl FixtureFile {
    pub fn generate(spec: &FixtureSpec) -> io::Result<Self> {
        if spec.target_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "benchmark fixture target must be non-zero",
            ));
        }
        let (path, file) = create_fixture(&spec.name)?;
        let mut writer = FixtureWriter::new(file);
        let mut line_count = 0_u64;
        let mut content_bytes = 0_u64;
        let mut lf_lines = 0_u64;
        let mut crlf_lines = 0_u64;

        match spec.shape {
            FixtureShape::Representative {
                average_physical_bytes,
                terminator,
            } => {
                let requested_lines = (spec.target_bytes / average_physical_bytes).max(1);
                let rotation = (spec.seed % 5) as usize;
                const DELTAS: [i64; 5] = [-8, -4, 0, 4, 8];
                for line in 0..requested_lines {
                    let delta = DELTAS[(line as usize + rotation) % DELTAS.len()];
                    let physical_bytes = average_physical_bytes
                        .checked_add_signed(delta)
                        .expect("representative physical length remains positive");
                    let line_content_bytes = physical_bytes - terminator.bytes().len() as u64;
                    writer.write_content(spec.seed, line, line_content_bytes)?;
                    writer.write_terminator(terminator)?;
                    line_count += 1;
                    content_bytes += line_content_bytes;
                    match terminator {
                        TerminatorStyle::Lf => lf_lines += 1,
                        TerminatorStyle::CrLf => crlf_lines += 1,
                    }
                }
            }
            FixtureShape::NewlineDense => {
                for _ in 0..spec.target_bytes {
                    writer.write_terminator(TerminatorStyle::Lf)?;
                }
                line_count = spec.target_bytes;
                lf_lines = spec.target_bytes;
            }
            FixtureShape::HugeLine {
                content_bytes: bytes,
            } => {
                writer.write_content(spec.seed, 0, bytes)?;
                writer.write_terminator(TerminatorStyle::Lf)?;
                line_count = 1;
                content_bytes = bytes;
                lf_lines = 1;
            }
        }

        let (length, physical_checksum, content_checksum) = writer.finish()?;
        Ok(Self {
            path,
            metadata: FixtureMetadata {
                name: spec.name.clone(),
                seed: spec.seed,
                length,
                line_count,
                content_bytes,
                lf_lines,
                crlf_lines,
                physical_checksum,
                content_checksum,
            },
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub const fn metadata(&self) -> &FixtureMetadata {
        &self.metadata
    }

    pub const fn len(&self) -> u64 {
        self.metadata.length
    }
}

impl Drop for FixtureFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

struct FixtureWriter {
    file: File,
    buffer: Vec<u8>,
    length: u64,
    physical_checksum: u64,
    content_checksum: u64,
}

impl FixtureWriter {
    fn new(file: File) -> Self {
        Self {
            file,
            buffer: Vec::with_capacity(FIXTURE_WRITE_BUFFER_BYTES),
            length: 0,
            physical_checksum: FNV_OFFSET_BASIS,
            content_checksum: FNV_OFFSET_BASIS,
        }
    }

    fn write_content(&mut self, seed: u64, line: u64, bytes: u64) -> io::Result<()> {
        for index in 0..bytes {
            let byte = fixture_content_byte(seed, line, index);
            self.push(byte, true)?;
        }
        Ok(())
    }

    fn write_terminator(&mut self, terminator: TerminatorStyle) -> io::Result<()> {
        for &byte in terminator.bytes() {
            self.push(byte, false)?;
        }
        Ok(())
    }

    fn push(&mut self, byte: u8, content: bool) -> io::Result<()> {
        self.buffer.push(byte);
        self.length += 1;
        update_checksum(&mut self.physical_checksum, &[byte]);
        if content {
            update_checksum(&mut self.content_checksum, &[byte]);
        }
        if self.buffer.len() == self.buffer.capacity() {
            self.flush_buffer()?;
        }
        Ok(())
    }

    fn flush_buffer(&mut self) -> io::Result<()> {
        self.file.write_all(&self.buffer)?;
        self.buffer.clear();
        Ok(())
    }

    fn finish(mut self) -> io::Result<(u64, u64, u64)> {
        self.flush_buffer()?;
        self.file.sync_all()?;
        Ok((self.length, self.physical_checksum, self.content_checksum))
    }
}

fn create_fixture(kind: &str) -> io::Result<(PathBuf, File)> {
    loop {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "faultsift-fs009-{kind}-{}-{id}.log",
            std::process::id()
        ));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

fn fixture_content_byte(seed: u64, line: u64, index: u64) -> u8 {
    let mixed = seed
        .wrapping_add(line.wrapping_mul(0x9e37_79b9_7f4a_7c15))
        .wrapping_add(index.wrapping_mul(131));
    b'a' + ((mixed ^ (mixed >> 17) ^ (mixed >> 41)) % 26) as u8
}

pub fn stream_file_checksum(path: &Path) -> io::Result<(u64, u64)> {
    let mut file = File::open(path)?;
    let mut buffer = [0_u8; 8 * KIB as usize];
    let mut bytes = 0_u64;
    let mut checksum = FNV_OFFSET_BASIS;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok((bytes, checksum));
        }
        bytes += read as u64;
        update_checksum(&mut checksum, &buffer[..read]);
    }
}

pub fn update_checksum(checksum: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *checksum ^= u64::from(*byte);
        *checksum = checksum.wrapping_mul(FNV_PRIME);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendMode {
    Automatic,
    #[cfg(windows)]
    ForcedBuffered,
}

impl BackendMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            #[cfg(windows)]
            Self::ForcedBuffered => "forced-buffered",
        }
    }

    const fn benchmark_id(self) -> &'static str {
        match self {
            #[cfg(windows)]
            Self::Automatic => "map",
            #[cfg(target_os = "linux")]
            Self::Automatic => "buf",
            #[cfg(windows)]
            Self::ForcedBuffered => "fbuf",
        }
    }
}

pub fn criterion_group_identity(api: &str, mode: BackendMode, config: BenchmarkConfig) -> String {
    assert!(api.len() <= 4, "benchmark API identity must stay compact");
    let identity = format!(
        "{api}-{}-{}-c{CRITERION_VERSION}-f{}-h{}-q{}-s{RANDOM_SEED:016x}-warm",
        mode.benchmark_id(),
        if config.smoke { "smk" } else { "full" },
        config.representative_bytes / MIB,
        config.huge_line_bytes / MIB,
        config.lookup_requests,
    );
    assert!(
        identity.len() <= 64,
        "Criterion group identity must not be truncated"
    );
    identity
}

pub fn backend_modes() -> &'static [BackendMode] {
    #[cfg(windows)]
    {
        &[BackendMode::ForcedBuffered, BackendMode::Automatic]
    }
    #[cfg(target_os = "linux")]
    {
        &[BackendMode::Automatic]
    }
}

pub fn open_snapshot(path: &Path, mode: BackendMode) -> io::Result<Arc<FileSnapshot>> {
    #[cfg(windows)]
    let forcing_writer = match mode {
        BackendMode::Automatic => None,
        BackendMode::ForcedBuffered => Some(OpenOptions::new().write(true).open(path)?),
    };

    #[cfg(not(windows))]
    let _ = mode;

    let snapshot =
        FileSnapshot::open(path, FileAccessOptions::default()).map_err(io::Error::other)?;
    ensure_expected_backend(mode, snapshot.diagnostics())?;

    #[cfg(windows)]
    drop(forcing_writer);

    Ok(Arc::new(snapshot))
}

fn ensure_expected_backend(
    mode: BackendMode,
    diagnostics: FileAccessDiagnostics,
) -> io::Result<()> {
    #[cfg(windows)]
    match mode {
        BackendMode::Automatic if diagnostics.backend() != BackendKind::Mapped => {
            return Err(io::Error::other(format!(
                "automatic Windows mapping unavailable: {diagnostics:?}"
            )));
        }
        BackendMode::ForcedBuffered
            if diagnostics.backend() != BackendKind::Buffered
                || diagnostics.mapping_fallback_reason()
                    != Some(MappingFallbackReason::IncompatibleWriter) =>
        {
            return Err(io::Error::other(format!(
                "forced buffered backend was not proven: {diagnostics:?}"
            )));
        }
        _ => {}
    }

    #[cfg(target_os = "linux")]
    if mode != BackendMode::Automatic || diagnostics.backend() != BackendKind::Buffered {
        return Err(io::Error::other(format!(
            "Linux benchmark requires automatic buffered access: {diagnostics:?}"
        )));
    }

    Ok(())
}

pub fn backend_diagnostics(snapshot: &FileSnapshot) -> String {
    format!(
        "backend={:?},fallback={:?}",
        snapshot.diagnostics().backend(),
        snapshot.diagnostics().mapping_fallback_reason()
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConsumeOutcome {
    pub content_bytes: u64,
    pub physical_bytes: u64,
    pub chunks: u64,
    pub lines: u64,
    pub lf_lines: u64,
    pub crlf_lines: u64,
    pub unterminated_lines: u64,
    pub checksum: u64,
}

impl ConsumeOutcome {
    fn empty() -> Self {
        Self {
            content_bytes: 0,
            physical_bytes: 0,
            chunks: 0,
            lines: 0,
            lf_lines: 0,
            crlf_lines: 0,
            unterminated_lines: 0,
            checksum: FNV_OFFSET_BASIS,
        }
    }
}

pub fn run_cursor(
    snapshot: Arc<FileSnapshot>,
    scan_chunk_bytes: u64,
) -> io::Result<ConsumeOutcome> {
    let options = ScanOptions::new(ByteLength::new(scan_chunk_bytes)).map_err(io::Error::other)?;
    let mut cursor = PhysicalLineCursor::new(snapshot, options).map_err(io::Error::other)?;
    let mut outcome = ConsumeOutcome::empty();
    loop {
        let descriptor = cursor
            .visit_next_line(|chunk| {
                outcome.content_bytes += chunk.bytes().len() as u64;
                outcome.chunks += 1;
                update_checksum(&mut outcome.checksum, chunk.bytes());
                Ok::<(), io::Error>(())
            })
            .map_err(io::Error::other)?;
        let Some(descriptor) = descriptor else {
            return Ok(outcome);
        };
        outcome.lines += 1;
        outcome.physical_bytes += descriptor.physical_range().length().get();
        match descriptor.terminator() {
            LineTerminator::Lf => outcome.lf_lines += 1,
            LineTerminator::CrLf => outcome.crlf_lines += 1,
            LineTerminator::None => outcome.unterminated_lines += 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IndexEvidence {
    pub line_count: u64,
    pub final_stride: u64,
    pub checkpoint_count: u64,
    pub checkpoint_capacity: u64,
    pub checkpoint_payload_bytes: u64,
    pub checkpoint_capacity_bytes: u64,
    pub compaction_count: u32,
    pub checkpoint_budget_bytes: u64,
    pub scan_chunk_bytes: u64,
}

pub fn index_options(checkpoint_budget_bytes: u64, scan_chunk_bytes: u64) -> LineIndexOptions {
    LineIndexOptions::new(
        ByteLength::new(checkpoint_budget_bytes),
        ByteLength::new(scan_chunk_bytes),
    )
    .expect("benchmark resource candidates must be valid")
}

pub fn index_evidence(index: &LineIndex) -> IndexEvidence {
    let checkpoint_capacity = index.checkpoint_budget_bytes().get() / 8;
    let mut stride = index.final_stride();
    let mut compaction_count = 0_u32;
    while stride > 256 {
        assert!(stride.is_multiple_of(2));
        stride /= 2;
        compaction_count += 1;
    }
    assert_eq!(stride, 256);
    IndexEvidence {
        line_count: index.line_count(),
        final_stride: index.final_stride(),
        checkpoint_count: index.checkpoint_count(),
        checkpoint_capacity,
        checkpoint_payload_bytes: index.checkpoint_count() * 8,
        checkpoint_capacity_bytes: checkpoint_capacity * 8,
        compaction_count,
        checkpoint_budget_bytes: index.checkpoint_budget_bytes().get(),
        scan_chunk_bytes: index.scan_chunk_bytes().get(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CancellationEvidence {
    pub callback_count: u64,
    pub bytes_scanned: u64,
    pub physical_lines_completed: u64,
    pub final_stride: u64,
    pub checkpoint_count: u64,
    pub detection_to_return_ns: u128,
}

pub fn cancellation_at_callback(
    snapshot: Arc<FileSnapshot>,
    checkpoint_budget_bytes: u64,
    scan_chunk_bytes: u64,
    cancel_at: u64,
) -> io::Result<CancellationEvidence> {
    let mut observed = None;
    let mut detected_at = None;
    let mut callbacks = 0_u64;
    let result = LineIndex::build_with_control(
        snapshot,
        index_options(checkpoint_budget_bytes, scan_chunk_bytes),
        |progress| {
            callbacks += 1;
            if callbacks == cancel_at {
                observed = Some(CancellationEvidence {
                    callback_count: callbacks,
                    bytes_scanned: progress.bytes_scanned().get(),
                    physical_lines_completed: progress.physical_lines_completed(),
                    final_stride: progress.current_stride(),
                    checkpoint_count: progress.checkpoint_count(),
                    detection_to_return_ns: 0,
                });
                detected_at = Some(Instant::now());
                BuildControl::Cancel
            } else {
                BuildControl::Continue
            }
        },
    );
    if !matches!(result, Err(LineAccessError::IndexBuildCancelled)) {
        return Err(io::Error::other("benchmark cancellation did not cancel"));
    }
    let mut evidence = observed
        .ok_or_else(|| io::Error::other("requested cancellation callback was not reached"))?;
    evidence.detection_to_return_ns = detected_at
        .expect("cancel detection time accompanies evidence")
        .elapsed()
        .as_nanos();
    Ok(evidence)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineRequest {
    pub line_number: LineNumber,
    pub expected: LineDescriptor,
    pub scanned_lines: u64,
    pub scanned_bytes: u64,
}

pub fn generate_line_numbers(line_count: u64, count: usize, seed: u64) -> Vec<LineNumber> {
    assert!(line_count > 0);
    let mut random = FixedRandom::new(seed);
    (0..count)
        .map(|_| LineNumber::new(random.next_u64() % line_count))
        .collect()
}

pub fn prepare_line_requests(
    index: &LineIndex,
    numbers: &[LineNumber],
) -> io::Result<Vec<LineRequest>> {
    numbers
        .iter()
        .copied()
        .map(|line_number| {
            let expected = index.line(line_number).map_err(io::Error::other)?;
            let checkpoint_line = (line_number.get() / index.final_stride()) * index.final_stride();
            let checkpoint = index
                .line(LineNumber::new(checkpoint_line))
                .map_err(io::Error::other)?;
            Ok(LineRequest {
                line_number,
                expected,
                scanned_lines: line_number.get() - checkpoint_line + 1,
                scanned_bytes: expected.physical_range().end().get()
                    - checkpoint.physical_range().offset().get(),
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LookupOutcome {
    pub operations: u64,
    pub scanned_lines: u64,
    pub scanned_bytes: u64,
    pub checksum: u64,
}

pub fn run_line_requests(index: &LineIndex, requests: &[LineRequest]) -> io::Result<LookupOutcome> {
    let mut outcome = LookupOutcome {
        operations: 0,
        scanned_lines: 0,
        scanned_bytes: 0,
        checksum: 0,
    };
    for request in requests {
        let actual = index.line(request.line_number).map_err(io::Error::other)?;
        if actual != request.expected {
            return Err(io::Error::other(
                "ready line lookup changed exact coordinates",
            ));
        }
        outcome.operations += 1;
        outcome.scanned_lines += request.scanned_lines;
        outcome.scanned_bytes += request.scanned_bytes;
        outcome.checksum = outcome.checksum.rotate_left(7)
            ^ actual.physical_range().offset().get()
            ^ actual.physical_range().end().get();
    }
    Ok(outcome)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RangeRequest {
    pub range: LineRange,
    pub expected: LineSpan,
}

pub fn prepare_range_requests(
    index: &LineIndex,
    ranges: &[LineRange],
) -> io::Result<Vec<RangeRequest>> {
    ranges
        .iter()
        .copied()
        .map(|range| {
            Ok(RangeRequest {
                range,
                expected: index.line_range(range).map_err(io::Error::other)?,
            })
        })
        .collect()
}

pub fn run_range_requests(index: &LineIndex, requests: &[RangeRequest]) -> io::Result<u64> {
    let mut checksum = 0_u64;
    for request in requests {
        let actual = index.line_range(request.range).map_err(io::Error::other)?;
        if actual != request.expected {
            return Err(io::Error::other(
                "ready range lookup changed exact coordinates",
            ));
        }
        checksum = checksum.rotate_left(9)
            ^ actual.physical_range().offset().get()
            ^ actual.physical_range().end().get()
            ^ actual.line_range().start().get()
            ^ actual.line_range().end().get();
    }
    Ok(checksum)
}

pub fn seeded_ranges(line_count: u64, count: usize, span_lines: u64, seed: u64) -> Vec<LineRange> {
    assert!(line_count > 0);
    let mut random = FixedRandom::new(seed);
    (0..count)
        .map(|_| {
            let maximum_start = line_count.saturating_sub(span_lines);
            let start = if maximum_start == 0 {
                0
            } else {
                random.next_u64() % (maximum_start + 1)
            };
            let end = (start + span_lines).min(line_count);
            LineRange::new(LineNumber::new(start), LineNumber::new(end)).unwrap()
        })
        .collect()
}

pub fn empty_ranges(line_count: u64, count: usize, seed: u64) -> Vec<LineRange> {
    let mut random = FixedRandom::new(seed);
    (0..count)
        .map(|_| {
            let anchor = random.next_u64() % (line_count + 1);
            LineRange::new(LineNumber::new(anchor), LineNumber::new(anchor)).unwrap()
        })
        .collect()
}

pub fn checkpoint_crossing_ranges(
    line_count: u64,
    final_stride: u64,
    count: usize,
) -> Vec<LineRange> {
    assert!(line_count > 0);
    let checkpoint_count = line_count.div_ceil(final_stride).max(1);
    (0..count)
        .map(|ordinal| {
            let boundary = ((ordinal as u64 % checkpoint_count) * final_stride).min(line_count);
            let start = boundary.saturating_sub(2);
            let end = (boundary + 3).min(line_count);
            LineRange::new(LineNumber::new(start), LineNumber::new(end)).unwrap()
        })
        .collect()
}

pub fn visit_line(index: &LineIndex, descriptor: &LineDescriptor) -> io::Result<ConsumeOutcome> {
    let mut outcome = ConsumeOutcome::empty();
    index
        .visit_line_content(descriptor, |chunk| {
            outcome.content_bytes += chunk.bytes().len() as u64;
            outcome.chunks += 1;
            update_checksum(&mut outcome.checksum, chunk.bytes());
            Ok::<(), io::Error>(())
        })
        .map_err(io::Error::other)?;
    outcome.physical_bytes = descriptor.physical_range().length().get();
    outcome.lines = 1;
    Ok(outcome)
}

pub fn visit_span(index: &LineIndex, span: &LineSpan) -> io::Result<ConsumeOutcome> {
    let mut outcome = ConsumeOutcome::empty();
    index
        .visit_span_physical(span, |chunk| {
            outcome.physical_bytes += chunk.bytes().len() as u64;
            outcome.chunks += 1;
            update_checksum(&mut outcome.checksum, chunk.bytes());
            Ok::<(), io::Error>(())
        })
        .map_err(io::Error::other)?;
    outcome.lines = span.line_range().end().get() - span.line_range().start().get();
    Ok(outcome)
}

#[derive(Clone, Copy, Debug)]
struct FixedRandom(u64);

impl FixedRandom {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value >> 12;
        value ^= value << 25;
        value ^= value >> 27;
        self.0 = value;
        value.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BenchmarkConfig {
    pub smoke: bool,
    pub representative_bytes: u64,
    pub huge_line_bytes: u64,
    pub lookup_requests: usize,
}

impl BenchmarkConfig {
    pub fn from_environment() -> Self {
        let smoke = environment_flag("FAULTSIFT_LINE_BENCH_SMOKE");
        let representative_mib = bounded_mib_override(
            env::var("FAULTSIFT_LINE_BENCH_FIXTURE_MIB").ok().as_deref(),
            if smoke { 2 } else { 16 },
            1,
            MAX_REPRESENTATIVE_FIXTURE_MIB,
            "FAULTSIFT_LINE_BENCH_FIXTURE_MIB",
        )
        .expect("representative fixture override must be bounded");
        let huge_mib = bounded_mib_override(
            env::var("FAULTSIFT_LINE_BENCH_HUGE_MIB").ok().as_deref(),
            if smoke { 2 } else { 16 },
            1,
            MAX_HUGE_LINE_MIB,
            "FAULTSIFT_LINE_BENCH_HUGE_MIB",
        )
        .expect("huge-line override must be bounded");
        let lookup_requests = bounded_usize_override(
            env::var("FAULTSIFT_LINE_BENCH_LOOKUPS").ok().as_deref(),
            if smoke { 32 } else { 256 },
            8,
            MAX_LOOKUP_REQUESTS,
            "FAULTSIFT_LINE_BENCH_LOOKUPS",
        )
        .expect("lookup request override must be bounded");
        Self {
            smoke,
            representative_bytes: representative_mib * MIB,
            huge_line_bytes: huge_mib * MIB,
            lookup_requests,
        }
    }

    pub fn representative_specs(self) -> Vec<FixtureSpec> {
        let mut specs = Vec::new();
        for average in [80, 200, 500] {
            for terminator in [TerminatorStyle::Lf, TerminatorStyle::CrLf] {
                specs.push(FixtureSpec::representative(
                    average,
                    terminator,
                    self.representative_bytes,
                    RANDOM_SEED,
                ));
            }
        }
        specs
    }
}

pub fn bounded_mib_override(
    raw: Option<&str>,
    default_mib: u64,
    minimum_mib: u64,
    maximum_mib: u64,
    name: &str,
) -> Result<u64, String> {
    let value = match raw {
        Some(raw) => raw
            .parse::<u64>()
            .map_err(|_| format!("{name} must be an integer MiB value"))?,
        None => default_mib,
    };
    if !(minimum_mib..=maximum_mib).contains(&value) {
        return Err(format!(
            "{name} must be between {minimum_mib} and {maximum_mib} MiB"
        ));
    }
    Ok(value)
}

fn bounded_usize_override(
    raw: Option<&str>,
    default: usize,
    minimum: usize,
    maximum: usize,
    name: &str,
) -> Result<usize, String> {
    let value = match raw {
        Some(raw) => raw
            .parse::<usize>()
            .map_err(|_| format!("{name} must be an integer"))?,
        None => default,
    };
    if !(minimum..=maximum).contains(&value) {
        return Err(format!("{name} must be between {minimum} and {maximum}"));
    }
    Ok(value)
}

fn environment_flag(name: &str) -> bool {
    env::var_os(name).is_some_and(|value| value != "0")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessMemory {
    pub resident_bytes: Option<u64>,
    pub virtual_bytes: Option<u64>,
}

pub fn process_memory() -> ProcessMemory {
    #[cfg(windows)]
    {
        let command = format!(
            "$p=Get-Process -Id {}; Write-Output ($p.WorkingSet64.ToString() + ',' + $p.VirtualMemorySize64.ToString())",
            std::process::id()
        );
        let values = command_output(
            "powershell.exe",
            ["-NoProfile", "-NoLogo", "-Command", &command],
        )
        .and_then(|line| {
            let mut fields = line.trim().split(',');
            Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
        });
        ProcessMemory {
            resident_bytes: values.map(|value| value.0),
            virtual_bytes: values.map(|value| value.1),
        }
    }

    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string("/proc/self/status").ok();
        ProcessMemory {
            resident_bytes: status
                .as_deref()
                .and_then(|text| proc_status_bytes(text, "VmRSS:")),
            virtual_bytes: status
                .as_deref()
                .and_then(|text| proc_status_bytes(text, "VmSize:")),
        }
    }
}

#[cfg(target_os = "linux")]
fn proc_status_bytes(status: &str, key: &str) -> Option<u64> {
    status
        .lines()
        .find(|line| line.starts_with(key))?
        .split_ascii_whitespace()
        .nth(1)?
        .parse::<u64>()
        .ok()?
        .checked_mul(KIB)
}

pub fn print_memory(label: &str) {
    let memory = process_memory();
    println!(
        "[faultsift-line-benchmark-memory] label={label} rss_bytes={} virtual_bytes={}",
        display_optional(memory.resident_bytes),
        display_optional(memory.virtual_bytes)
    );
}

pub fn print_environment_metadata(config: BenchmarkConfig, fixture: &FixtureFile) {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commit = command_output(
        "git",
        [
            "-C",
            manifest_dir.to_string_lossy().as_ref(),
            "rev-parse",
            "HEAD",
        ],
    )
    .unwrap_or_else(|| "unknown".to_owned());
    let dirty_output = Command::new("git")
        .args([
            "-C",
            manifest_dir.to_string_lossy().as_ref(),
            "status",
            "--porcelain",
            "--untracked-files=normal",
        ])
        .output()
        .ok();
    let dirty = dirty_output
        .as_ref()
        .is_none_or(|output| !output.status.success() || !output.stdout.is_empty());
    let rustc = command_output("rustc", ["--version"]).unwrap_or_else(|| "unknown".to_owned());

    #[cfg(windows)]
    let os_detail = command_output("cmd.exe", ["/C", "ver"]);
    #[cfg(target_os = "linux")]
    let os_detail = command_output("uname", ["-srv"]);

    #[cfg(windows)]
    let cpu = command_output(
        "powershell.exe",
        [
            "-NoProfile",
            "-NoLogo",
            "-Command",
            "(Get-CimInstance Win32_Processor | Select-Object -First 1 -ExpandProperty Name)",
        ],
    )
    .or_else(|| env::var("PROCESSOR_IDENTIFIER").ok());
    #[cfg(target_os = "linux")]
    let cpu = fs::read_to_string("/proc/cpuinfo").ok().and_then(|text| {
        text.lines()
            .find_map(|line| line.strip_prefix("model name\t: ").map(ToOwned::to_owned))
    });

    let filesystem = filesystem_metadata(fixture.path());
    let storage = env::var("FAULTSIFT_LINE_BENCH_STORAGE_CLASS")
        .ok()
        .or_else(|| storage_metadata(fixture.path()))
        .unwrap_or_else(|| "not-collected; record manually for formal runs".to_owned());

    println!("\n[faultsift-line-benchmark-metadata]");
    println!("commit_sha={}", commit.trim());
    println!("worktree_dirty={dirty}");
    println!("worktree_dirty_policy=staged+unstaged+untracked");
    println!(
        "os={} {}",
        env::consts::OS,
        os_detail.as_deref().unwrap_or("unknown").trim()
    );
    println!("architecture={}", env::consts::ARCH);
    println!("cpu={}", cpu.as_deref().unwrap_or("unknown").trim());
    println!(
        "system_memory_bytes={}",
        display_optional(total_system_memory())
    );
    println!(
        "filesystem={}",
        filesystem.as_deref().unwrap_or("unknown").trim()
    );
    println!("storage={storage}");
    println!("rustc={}", rustc.trim());
    println!("benchmark_profile=bench/optimized");
    println!("benchmark_tool=criterion {CRITERION_VERSION}");
    println!("cache_condition=warm-cache; OS page cache uncontrolled");
    println!("cold_cache=deferred; no system cache flush is performed");
    println!("cpu_time=not-collected");
    println!("fixture_seed=0x{RANDOM_SEED:016x}");
    println!("fixture_size_bytes={}", config.representative_bytes);
    println!("huge_line_content_bytes={}", config.huge_line_bytes);
    println!("lookup_requests={}", config.lookup_requests);
    println!("smoke_mode={}", config.smoke);
    println!("benchmark_concurrency_levels={BENCHMARK_CONCURRENCY_LEVELS:?}");
}

fn display_optional(value: Option<u64>) -> String {
    value.map_or_else(|| "not-collected".to_owned(), |value| value.to_string())
}

fn command_output<I, S>(program: &str, args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(program).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(windows)]
fn filesystem_metadata(path: &Path) -> Option<String> {
    let Component::Prefix(prefix) = path.components().next()? else {
        return None;
    };
    let root = format!("{}\\", prefix.as_os_str().to_string_lossy());
    command_output(
        "fsutil.exe",
        [
            OsStr::new("fsinfo"),
            OsStr::new("volumeinfo"),
            root.as_ref(),
        ],
    )
    .map(|output| output.lines().collect::<Vec<_>>().join(" | "))
    .or_else(|| {
        let drive_letter = prefix.as_os_str().to_string_lossy().chars().next()?;
        let command = format!(
            "$v=Get-Volume -DriveLetter '{drive_letter}'; Write-Output ($v.FileSystem + ',' + $v.DriveType + ',' + $v.Size)"
        );
        command_output(
            "powershell.exe",
            ["-NoProfile", "-NoLogo", "-Command", &command],
        )
    })
}

#[cfg(target_os = "linux")]
fn filesystem_metadata(path: &Path) -> Option<String> {
    command_output("df", [OsStr::new("-T"), path.as_os_str()])
        .and_then(|output| output.lines().nth(1).map(ToOwned::to_owned))
}

#[cfg(windows)]
fn storage_metadata(path: &Path) -> Option<String> {
    let Component::Prefix(prefix) = path.components().next()? else {
        return None;
    };
    let drive_letter = prefix.as_os_str().to_string_lossy().chars().next()?;
    let command = format!(
        "$d=Get-Partition -DriveLetter '{drive_letter}' | Get-Disk; Write-Output ($d.FriendlyName + ',' + $d.BusType)"
    );
    command_output(
        "powershell.exe",
        ["-NoProfile", "-NoLogo", "-Command", &command],
    )
}

#[cfg(target_os = "linux")]
fn storage_metadata(_path: &Path) -> Option<String> {
    None
}

#[cfg(windows)]
fn total_system_memory() -> Option<u64> {
    command_output(
        "powershell.exe",
        [
            "-NoProfile",
            "-NoLogo",
            "-Command",
            "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
        ],
    )?
    .trim()
    .parse()
    .ok()
}

#[cfg(target_os = "linux")]
fn total_system_memory() -> Option<u64> {
    let memory = fs::read_to_string("/proc/meminfo").ok()?;
    proc_status_bytes(&memory, "MemTotal:")
}

impl fmt::Display for FixtureMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "fixture={} bytes={} lines={} average_physical_bytes={:.3} content_bytes={} lf_lines={} crlf_lines={} seed=0x{:016x}",
            self.name,
            self.length,
            self.line_count,
            self.average_physical_bytes(),
            self.content_bytes,
            self.lf_lines,
            self.crlf_lines,
            self.seed
        )
    }
}
