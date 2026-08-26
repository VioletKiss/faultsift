use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};

const FRESH: u8 = 0;

/// First confirmed reason that a snapshot can no longer be treated as fresh.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
#[repr(u8)]
pub enum StaleReason {
    Grown = 1,
    Truncated = 2,
    Modified = 3,
    Replaced = 4,
    Missing = 5,
    Unverifiable = 6,
    UnexpectedEof = 7,
}

impl StaleReason {
    fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Grown,
            2 => Self::Truncated,
            3 => Self::Modified,
            4 => Self::Replaced,
            5 => Self::Missing,
            6 => Self::Unverifiable,
            7 => Self::UnexpectedEof,
            _ => unreachable!("snapshot lifecycle contains an invalid state"),
        }
    }
}

impl fmt::Display for StaleReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Grown => "source file grew",
            Self::Truncated => "source file was truncated",
            Self::Modified => "source file metadata changed",
            Self::Replaced => "path resolves to a different file",
            Self::Missing => "source path is missing",
            Self::Unverifiable => "source state could not be verified",
            Self::UnexpectedEof => "source reached EOF before the snapshot boundary",
        })
    }
}

/// Current one-way lifecycle state of a snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotState {
    Fresh,
    Stale(StaleReason),
}

/// Result of an explicit snapshot validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotValidation {
    Unchanged,
    Stale(StaleReason),
}

/// Metadata target that could not be inspected during explicit validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationTarget {
    OpenFile,
    CurrentPath,
}

impl fmt::Display for ValidationTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OpenFile => "the opened file",
            Self::CurrentPath => "the current path target",
        })
    }
}

#[derive(Debug)]
pub(crate) struct SnapshotLifecycle {
    state: AtomicU8,
}

impl SnapshotLifecycle {
    pub(crate) const fn fresh() -> Self {
        Self {
            state: AtomicU8::new(FRESH),
        }
    }

    pub(crate) fn state(&self) -> SnapshotState {
        match self.state.load(Ordering::Acquire) {
            FRESH => SnapshotState::Fresh,
            code => SnapshotState::Stale(StaleReason::from_code(code)),
        }
    }

    pub(crate) fn stale_reason(&self) -> Option<StaleReason> {
        match self.state() {
            SnapshotState::Fresh => None,
            SnapshotState::Stale(reason) => Some(reason),
        }
    }

    pub(crate) fn mark_stale(&self, reason: StaleReason) -> StaleReason {
        match self
            .state
            .compare_exchange(FRESH, reason as u8, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => reason,
            Err(existing) => StaleReason::from_code(existing),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_stale_reason_wins_permanently() {
        for reason in [
            StaleReason::Grown,
            StaleReason::Truncated,
            StaleReason::Modified,
            StaleReason::Replaced,
            StaleReason::Missing,
            StaleReason::Unverifiable,
            StaleReason::UnexpectedEof,
        ] {
            let lifecycle = SnapshotLifecycle::fresh();
            assert_eq!(lifecycle.state(), SnapshotState::Fresh);
            assert_eq!(lifecycle.mark_stale(reason), reason);
            assert_eq!(lifecycle.mark_stale(StaleReason::Replaced), reason);
            assert_eq!(lifecycle.state(), SnapshotState::Stale(reason));
        }
    }
}
