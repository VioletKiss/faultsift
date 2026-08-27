use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
#[cfg(windows)]
use std::path::Component;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(windows)]
use faultsift_file_access::MappingFallbackReason;
use faultsift_file_access::{
    BackendKind, ByteLength, ByteOffset, ByteRange, FileAccessDiagnostics, FileAccessOptions,
    FileSnapshot,
};

pub const KIB: u64 = 1024;
pub const MIB: u64 = 1024 * KIB;
pub const GIB: u64 = 1024 * MIB;
pub const RANDOM_SEED: u64 = 0x4653_3030_355f_5255;
pub const RANGE_SIZES: [u64; 4] = [4 * KIB, 64 * KIB, MIB, 8 * MIB];
pub const CONCURRENCY_LEVELS: [usize; 2] = [1, 4];
pub const BENCHMARK_MAX_VIEW_BYTES: u64 = 8 * MIB;
pub const FOUR_GIB: u64 = 4 * GIB;
pub const SPARSE_FILE_SIZE: u64 = FOUR_GIB + 64 * KIB;

pub const FIXTURE_WRITE_BUFFER_BYTES: usize = MIB as usize;
const BOUNDARY_SENTINEL_OFFSET: u64 = FOUR_GIB - 8;
const END_SENTINEL_OFFSET: u64 = SPARSE_FILE_SIZE - 16;
const BOUNDARY_SENTINEL: [u8; 16] = *b"FS005-4G-BOUND!!";
const END_SENTINEL: [u8; 16] = *b"FS005-SPARSE-END";
pub const CRITERION_VERSION: &str = "0.7.0";

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendMode {
    Automatic,
    #[cfg(windows)]
    ForcedBuffered,
}

pub struct BackendModeGuard {
    #[cfg(windows)]
    _writer: Option<File>,
}

impl BackendModeGuard {
    pub fn acquire(path: &Path, mode: BackendMode) -> io::Result<Self> {
        #[cfg(windows)]
        let writer = match mode {
            BackendMode::Automatic => None,
            BackendMode::ForcedBuffered => Some(OpenOptions::new().write(true).open(path)?),
        };

        #[cfg(not(windows))]
        let _ = (path, mode);

        Ok(Self {
            #[cfg(windows)]
            _writer: writer,
        })
    }
}

impl BackendMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            #[cfg(windows)]
            Self::ForcedBuffered => "forced-buffered",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessApi {
    View,
    ReadAt,
}

impl AccessApi {
    pub const fn label(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::ReadAt => "read_at",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessPattern {
    Sequential,
    Random,
}

impl AccessPattern {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::Random => "seeded-random",
        }
    }
}

#[derive(Debug)]
pub struct FixtureFile {
    path: PathBuf,
    length: u64,
}

impl FixtureFile {
    pub fn populated(length: u64) -> io::Result<Self> {
        if length == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "populated benchmark fixture must be non-empty",
            ));
        }

        let (mut fixture, mut file) = Self::create("populated")?;
        let mut buffer = vec![0_u8; FIXTURE_WRITE_BUFFER_BYTES];
        let mut offset = 0_u64;

        while offset < length {
            let remaining = length - offset;
            let chunk_length = usize::try_from(remaining.min(buffer.len() as u64))
                .expect("bounded fixture chunk fits usize");
            fill_fixture_bytes(&mut buffer[..chunk_length], offset);
            file.write_all(&buffer[..chunk_length])?;
            offset += chunk_length as u64;
        }
        file.sync_all()?;

        fixture.length = length;
        Ok(fixture)
    }

    pub fn sparse_boundary() -> io::Result<(Self, SparseVerification)> {
        let (mut fixture, mut file) = Self::create("sparse-boundary")?;
        let verification = prepare_sparse_file(&fixture.path, &file)?;
        file.set_len(SPARSE_FILE_SIZE)?;

        file.seek(SeekFrom::Start(BOUNDARY_SENTINEL_OFFSET))?;
        file.write_all(&BOUNDARY_SENTINEL)?;
        file.seek(SeekFrom::Start(END_SENTINEL_OFFSET))?;
        file.write_all(&END_SENTINEL)?;
        file.sync_all()?;

        let verification = verify_sparse_allocation(&fixture.path, &file, verification)?;

        fixture.length = SPARSE_FILE_SIZE;
        Ok((fixture, verification))
    }

    fn create(kind: &str) -> io::Result<(Self, File)> {
        loop {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "faultsift-fs005-{kind}-{}-{id}.bin",
                std::process::id()
            ));
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => return Ok((Self { path, length: 0 }, file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub const fn len(&self) -> u64 {
        self.length
    }

    pub fn verify_populated_bytes(&self, offset: u64, bytes: &[u8]) -> bool {
        bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| *byte == fixture_byte(offset + index as u64))
    }
}

impl Drop for FixtureFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SparseVerification {
    #[cfg(windows)]
    WindowsSparseFlag,
    #[cfg(target_os = "linux")]
    LinuxAllocatedBlocks { allocated_bytes: u64 },
}

impl fmt::Display for SparseVerification {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(windows)]
            Self::WindowsSparseFlag => formatter.write_str("Windows sparse-file flag confirmed"),
            #[cfg(target_os = "linux")]
            Self::LinuxAllocatedBlocks { allocated_bytes } => {
                write!(formatter, "Linux allocated blocks: {allocated_bytes} bytes")
            }
        }
    }
}

pub fn fixture_byte(offset: u64) -> u8 {
    let mixed = offset
        .wrapping_mul(131)
        .wrapping_add(RANDOM_SEED)
        .wrapping_add(offset >> 8);
    (mixed ^ (mixed >> 17) ^ (mixed >> 41)) as u8
}

fn fill_fixture_bytes(buffer: &mut [u8], offset: u64) {
    for (index, byte) in buffer.iter_mut().enumerate() {
        *byte = fixture_byte(offset + index as u64);
    }
}

#[cfg(windows)]
fn prepare_sparse_file(path: &Path, _file: &File) -> io::Result<SparseVerification> {
    let set_status = Command::new("fsutil.exe")
        .args([OsStr::new("sparse"), OsStr::new("setflag")])
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !set_status.success() {
        return Err(io::Error::other(format!(
            "fsutil could not enable sparse semantics for {}",
            path.display()
        )));
    }

    let query_status = Command::new("fsutil.exe")
        .args([OsStr::new("sparse"), OsStr::new("queryflag")])
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !query_status.success() {
        return Err(io::Error::other(format!(
            "fsutil could not confirm sparse semantics for {}",
            path.display()
        )));
    }

    Ok(SparseVerification::WindowsSparseFlag)
}

#[cfg(target_os = "linux")]
fn prepare_sparse_file(_path: &Path, _file: &File) -> io::Result<SparseVerification> {
    Ok(SparseVerification::LinuxAllocatedBlocks { allocated_bytes: 0 })
}

#[cfg(windows)]
fn verify_sparse_allocation(
    _path: &Path,
    _file: &File,
    verification: SparseVerification,
) -> io::Result<SparseVerification> {
    if verification == SparseVerification::WindowsSparseFlag {
        Ok(verification)
    } else {
        Err(io::Error::other("Windows sparse flag was not confirmed"))
    }
}

#[cfg(target_os = "linux")]
fn verify_sparse_allocation(
    _path: &Path,
    file: &File,
    _verification: SparseVerification,
) -> io::Result<SparseVerification> {
    use std::os::unix::fs::MetadataExt;

    let allocated_bytes = file.metadata()?.blocks().saturating_mul(512);
    if allocated_bytes >= SPARSE_FILE_SIZE / 16 {
        return Err(io::Error::other(format!(
            "filesystem did not create a bounded sparse fixture: {allocated_bytes} allocated bytes"
        )));
    }
    Ok(SparseVerification::LinuxAllocatedBlocks { allocated_bytes })
}

pub fn verify_sparse_snapshot(snapshot: &FileSnapshot) -> io::Result<u64> {
    if snapshot.len().get() != SPARSE_FILE_SIZE {
        return Err(io::Error::other("sparse snapshot length mismatch"));
    }

    let boundary_range = byte_range(BOUNDARY_SENTINEL_OFFSET, BOUNDARY_SENTINEL.len() as u64)?;
    let boundary = snapshot.view(boundary_range).map_err(io::Error::other)?;
    if boundary.as_bytes() != BOUNDARY_SENTINEL {
        return Err(io::Error::other("4 GiB boundary sentinel mismatch"));
    }

    let mut end = [0_u8; END_SENTINEL.len()];
    let read = snapshot
        .read_at(ByteOffset::new(END_SENTINEL_OFFSET), &mut end)
        .map_err(io::Error::other)?;
    if read != end.len() || end != END_SENTINEL {
        return Err(io::Error::other("sparse end sentinel mismatch"));
    }

    Ok((boundary.len() + read) as u64)
}

pub fn generate_ranges(
    file_size: u64,
    range_size: u64,
    total_bytes: u64,
    pattern: AccessPattern,
    seed: u64,
) -> io::Result<Vec<ByteRange>> {
    if range_size == 0 || range_size > file_size || total_bytes == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "range size and workload bytes must be non-zero and fit the fixture",
        ));
    }

    let operation_count = total_bytes.div_ceil(range_size).max(1);
    let capacity = usize::try_from(operation_count)
        .map_err(|_| io::Error::other("workload operation count does not fit usize"))?;
    let mut ranges = Vec::with_capacity(capacity);
    let mut random = FixedRandom::new(seed);
    let sequential_slots = file_size / range_size;
    let random_start_count = file_size - range_size + 1;

    for index in 0..operation_count {
        let offset = match pattern {
            AccessPattern::Sequential => (index % sequential_slots) * range_size,
            AccessPattern::Random => random.next_u64() % random_start_count,
        };
        ranges.push(byte_range(offset, range_size)?);
    }

    Ok(ranges)
}

pub fn total_range_bytes(ranges: &[ByteRange]) -> u64 {
    ranges.iter().map(|range| range.length().get()).sum()
}

fn byte_range(offset: u64, length: u64) -> io::Result<ByteRange> {
    ByteRange::new(ByteOffset::new(offset), ByteLength::new(length)).map_err(io::Error::other)
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

pub fn open_snapshot(
    path: &Path,
    max_view_bytes: u64,
    mode: BackendMode,
) -> io::Result<FileSnapshot> {
    let guard = BackendModeGuard::acquire(path, mode)?;
    open_snapshot_with_guard(path, max_view_bytes, mode, &guard)
}

pub fn open_snapshot_with_guard(
    path: &Path,
    max_view_bytes: u64,
    mode: BackendMode,
    _guard: &BackendModeGuard,
) -> io::Result<FileSnapshot> {
    let options =
        FileAccessOptions::new(ByteLength::new(max_view_bytes)).map_err(io::Error::other)?;
    let snapshot = FileSnapshot::open(path, options).map_err(io::Error::other)?;
    ensure_expected_backend(mode, snapshot.diagnostics())?;
    Ok(snapshot)
}

fn ensure_expected_backend(
    mode: BackendMode,
    diagnostics: FileAccessDiagnostics,
) -> io::Result<()> {
    #[cfg(windows)]
    match mode {
        BackendMode::Automatic if diagnostics.backend() != BackendKind::Mapped => {
            return Err(io::Error::other(format!(
                "automatic Windows mapping was required but selected {:?}: {:?}",
                diagnostics.backend(),
                diagnostics.mapping_fallback_reason()
            )));
        }
        BackendMode::ForcedBuffered
            if diagnostics.backend() != BackendKind::Buffered
                || diagnostics.mapping_fallback_reason()
                    != Some(MappingFallbackReason::IncompatibleWriter) =>
        {
            return Err(io::Error::other(format!(
                "forced buffered selection was not proven by diagnostics: {diagnostics:?}"
            )));
        }
        _ => {}
    }

    #[cfg(target_os = "linux")]
    {
        if mode != BackendMode::Automatic || diagnostics.backend() != BackendKind::Buffered {
            return Err(io::Error::other(format!(
                "Linux benchmark must use the automatic buffered backend: {diagnostics:?}"
            )));
        }
    }

    Ok(())
}

pub fn backend_modes() -> &'static [BackendMode] {
    #[cfg(windows)]
    {
        // Measure the forced path first. A live mapped snapshot intentionally
        // prevents the writer used to establish the forced-buffered case.
        &[BackendMode::ForcedBuffered, BackendMode::Automatic]
    }
    #[cfg(target_os = "linux")]
    {
        &[BackendMode::Automatic]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessOutcome {
    pub bytes: u64,
    pub checksum: u64,
}

pub fn run_view(snapshot: &FileSnapshot, ranges: &[ByteRange]) -> io::Result<AccessOutcome> {
    let mut outcome = AccessOutcome {
        bytes: 0,
        checksum: 0,
    };
    for range in ranges {
        let view = snapshot.view(*range).map_err(io::Error::other)?;
        outcome.bytes += view.len() as u64;
        if let Some(first) = view.as_bytes().first() {
            outcome.checksum = outcome.checksum.rotate_left(5) ^ u64::from(*first);
        }
        if let Some(last) = view.as_bytes().last() {
            outcome.checksum = outcome.checksum.rotate_left(7) ^ u64::from(*last);
        }
    }
    Ok(outcome)
}

pub fn run_read_at(
    snapshot: &FileSnapshot,
    ranges: &[ByteRange],
    buffer: &mut [u8],
) -> io::Result<AccessOutcome> {
    let mut outcome = AccessOutcome {
        bytes: 0,
        checksum: 0,
    };
    for range in ranges {
        let length = usize::try_from(range.length().get())
            .map_err(|_| io::Error::other("range length does not fit usize"))?;
        if buffer.len() < length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "caller buffer is smaller than the benchmark range",
            ));
        }
        let read = snapshot
            .read_at(range.offset(), &mut buffer[..length])
            .map_err(io::Error::other)?;
        if read != length {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "benchmark read did not fill the requested in-bounds range",
            ));
        }
        outcome.bytes += read as u64;
        outcome.checksum = outcome.checksum.rotate_left(5) ^ u64::from(buffer[0]);
        outcome.checksum = outcome.checksum.rotate_left(7) ^ u64::from(buffer[read - 1]);
    }
    Ok(outcome)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessMemory {
    pub resident_bytes: Option<u64>,
    pub virtual_bytes: Option<u64>,
    pub minor_faults: Option<u64>,
    pub major_faults: Option<u64>,
}

pub fn process_memory() -> ProcessMemory {
    #[cfg(windows)]
    {
        let command = format!(
            "$p=Get-Process -Id {}; Write-Output ($p.WorkingSet64.ToString() + ',' + $p.VirtualMemorySize64.ToString())",
            std::process::id()
        );
        let output = command_output(
            "powershell.exe",
            ["-NoProfile", "-NoLogo", "-Command", &command],
        );
        let values = output.as_deref().and_then(|line| {
            let mut fields = line.trim().split(',');
            Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
        });
        ProcessMemory {
            resident_bytes: values.map(|value| value.0),
            virtual_bytes: values.map(|value| value.1),
            minor_faults: None,
            major_faults: None,
        }
    }

    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string("/proc/self/status").ok();
        let resident_bytes = status
            .as_deref()
            .and_then(|text| proc_status_bytes(text, "VmRSS:"));
        let virtual_bytes = status
            .as_deref()
            .and_then(|text| proc_status_bytes(text, "VmSize:"));
        let (minor_faults, major_faults) = proc_faults().unwrap_or((None, None));
        ProcessMemory {
            resident_bytes,
            virtual_bytes,
            minor_faults,
            major_faults,
        }
    }
}

#[cfg(target_os = "linux")]
fn proc_status_bytes(status: &str, key: &str) -> Option<u64> {
    let value_kib = status
        .lines()
        .find(|line| line.starts_with(key))?
        .split_ascii_whitespace()
        .nth(1)?
        .parse::<u64>()
        .ok()?;
    value_kib.checked_mul(KIB)
}

#[cfg(target_os = "linux")]
fn proc_faults() -> Option<(Option<u64>, Option<u64>)> {
    let stat = fs::read_to_string("/proc/self/stat").ok()?;
    let after_name = stat.rsplit_once(')')?.1.trim();
    let fields: Vec<&str> = after_name.split_ascii_whitespace().collect();
    let minor = fields.get(7)?.parse().ok();
    let major = fields.get(9)?.parse().ok();
    Some((minor, major))
}

pub fn print_environment_metadata(fixture: &FixtureFile, smoke: bool) {
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
    let unstaged_clean = Command::new("git")
        .args([
            "-C",
            manifest_dir.to_string_lossy().as_ref(),
            "diff",
            "--quiet",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    let staged_clean = Command::new("git")
        .args([
            "-C",
            manifest_dir.to_string_lossy().as_ref(),
            "diff",
            "--cached",
            "--quiet",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    let dirty = !(unstaged_clean && staged_clean);
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
    let memory_total = total_system_memory();
    let storage_class = env::var("FAULTSIFT_BENCH_STORAGE_CLASS")
        .ok()
        .or_else(|| storage_metadata(fixture.path()))
        .unwrap_or_else(|| "not-collected; record manually for formal runs".to_owned());

    println!("\n[faultsift-benchmark-metadata]");
    println!("commit_sha={}", commit.trim());
    println!("tracked_worktree_dirty={dirty}");
    println!(
        "os={} {}",
        env::consts::OS,
        os_detail.as_deref().unwrap_or("unknown").trim()
    );
    println!("architecture={}", env::consts::ARCH);
    println!("cpu={}", cpu.as_deref().unwrap_or("unknown").trim());
    println!("system_memory_bytes={}", display_optional(memory_total));
    println!(
        "filesystem={}",
        filesystem.as_deref().unwrap_or("unknown").trim()
    );
    println!("storage_class={storage_class}");
    println!("rust={}", rustc.trim());
    println!("profile=bench/optimized");
    println!("benchmark_tool=criterion {CRITERION_VERSION}");
    println!("fixture_path={}", fixture.path().display());
    println!("fixture_size_bytes={}", fixture.len());
    println!("fixture_seed=0x{RANDOM_SEED:016x}");
    println!("random_seed=0x{RANDOM_SEED:016x}");
    println!("cache_condition=warm-cache; OS page cache is uncontrolled");
    println!("cold_cache=not measured; no system cache dropping is performed");
    println!("smoke_mode={smoke}");
    println!("max_live_view_bytes={BENCHMARK_MAX_VIEW_BYTES}");
    println!("concurrency_levels={CONCURRENCY_LEVELS:?}");
}

pub fn print_memory(label: &str, memory: &ProcessMemory) {
    println!(
        "[faultsift-benchmark-memory] label={label} rss_bytes={} virtual_bytes={} minor_faults={} major_faults={}",
        display_optional(memory.resident_bytes),
        display_optional(memory.virtual_bytes),
        display_optional(memory.minor_faults),
        display_optional(memory.major_faults)
    );
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
        .and_then(|output| output.lines().nth(1).map(|line| line.to_owned()))
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

pub fn mode_diagnostics(snapshot: &FileSnapshot) -> String {
    format!(
        "backend={:?},fallback={:?}",
        snapshot.diagnostics().backend(),
        snapshot.diagnostics().mapping_fallback_reason()
    )
}
