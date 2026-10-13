//! Job system and task graph.
//!
//! Ported 1:1 from `UHE/src/UHE/Jobsystem/` in the C++ engine. The two
//! memory-ordering invariants documented there are load-bearing and are
//! preserved verbatim in the comments below; both were found by ThreadSanitizer
//! or by reasoning about a missed wakeup.

mod jobs;
mod taskgraph;

pub use jobs::{Job, JobCounter, Jobs};
pub use taskgraph::{TaskGraph, TaskId};