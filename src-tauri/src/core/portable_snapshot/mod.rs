use super::{QuickCommandsStore, SessionManager};
use crate::config;
use crate::error::AppResult;
#[cfg(test)]
pub(crate) use nyaterm_core::core::portable_snapshot::test_support::encode_v3_raw_snapshot_redb_for_test;
pub use nyaterm_core::core::portable_snapshot::*;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
include!("build.rs");
include!("apply.rs");
