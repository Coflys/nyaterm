//! Owned runtime for watching files opened in an external editor.

use crate::models::external_editor::ExternalEditorFileVersion;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

use futures::channel::mpsc::UnboundedSender;

use crate::models::{TransferJobEvent, TransferJobResult};

const WATCH_INTERVAL: Duration = Duration::from_millis(1000);
const UPLOAD_SETTLE: Duration = Duration::from_millis(450);

pub(super) struct ExternalEditorWatcher {
    stop_tx: Option<mpsc::Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl ExternalEditorWatcher {
    pub(super) fn spawn(
        job_id: String,
        remote_path: String,
        raw_path_token: Option<String>,
        local_path: PathBuf,
        transfer_tx: UnboundedSender<TransferJobResult>,
        baseline: ExternalEditorFileVersion,
    ) -> std::io::Result<Self> {
        let (stop_tx, stop_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("nyaterm-external-editor-watch".to_string())
            .spawn(move || {
                watch_external_editor_file(
                    job_id,
                    remote_path,
                    raw_path_token,
                    local_path,
                    transfer_tx,
                    stop_rx,
                    WatchTimings::default(),
                    baseline,
                );
            })?;
        Ok(Self {
            stop_tx: Some(stop_tx),
            worker: Some(worker),
        })
    }

    pub(super) fn stop(&mut self) -> Option<JoinHandle<()>> {
        self.stop_tx.take();
        self.worker.take()
    }
}

impl Drop for ExternalEditorWatcher {
    fn drop(&mut self) {
        if let Some(worker) = self.stop() {
            // Session removal can happen on the UI thread while a file read is
            // outstanding. Request stop now, but never join there.
            let _ = thread::Builder::new()
                .name("nyaterm-editor-watch-reaper".into())
                .spawn(move || {
                    let _ = worker.join();
                });
        }
    }
}

#[derive(Clone, Copy)]
struct WatchTimings {
    interval: Duration,
    settle: Duration,
}

impl Default for WatchTimings {
    fn default() -> Self {
        Self {
            interval: WATCH_INTERVAL,
            settle: UPLOAD_SETTLE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalFileFingerprint {
    len: u64,
    modified: Option<SystemTime>,
}

impl LocalFileFingerprint {
    fn from_path(path: &Path) -> std::io::Result<Self> {
        let metadata = fs::metadata(path)?;
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn watch_external_editor_file(
    job_id: String,
    remote_path: String,
    raw_path_token: Option<String>,
    local_path: PathBuf,
    transfer_tx: UnboundedSender<TransferJobResult>,
    stop_rx: mpsc::Receiver<()>,
    timings: WatchTimings,
    mut baseline: ExternalEditorFileVersion,
) {
    let mut first_scan = true;
    loop {
        if wait_for_stop(
            &stop_rx,
            if first_scan {
                Duration::ZERO
            } else {
                timings.interval
            },
        ) {
            break;
        }
        let current = match LocalFileFingerprint::from_path(&local_path) {
            Ok(fingerprint) => fingerprint,
            // Atomic editor saves may temporarily remove the path.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if wait_for_stop(&stop_rx, timings.interval) {
                    break;
                }
                continue;
            }
            Err(error) => {
                report_watch_error(&transfer_tx, &job_id, error);
                break;
            }
        };
        if !first_scan && current.len == baseline.len && current.modified == baseline.modified {
            continue;
        }
        first_scan = false;
        if wait_for_stop(&stop_rx, timings.settle) {
            break;
        }
        let version = match read_external_editor_version(&local_path, || {
            wait_for_stop(&stop_rx, Duration::ZERO)
        }) {
            Ok(version) => version,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => break,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::NotFound
                ) =>
            {
                // Force another content check even if metadata returns to baseline.
                first_scan = true;
                if wait_for_stop(&stop_rx, timings.interval) {
                    break;
                }
                continue;
            }
            Err(error) => {
                report_watch_error(&transfer_tx, &job_id, error);
                break;
            }
        };
        if version.len != current.len || version.modified != current.modified {
            // A save during the settle window needs a full quiet interval.
            first_scan = true;
            continue;
        }
        if version.digest != baseline.digest
            && transfer_tx
                .unbounded_send(TransferJobResult {
                    id: job_id.clone(),
                    event: TransferJobEvent::ExternalModified {
                        remote_path: remote_path.clone(),
                        raw_path_token: raw_path_token.clone(),
                        local_path: local_path.clone(),
                    },
                })
                .is_err()
        {
            break;
        }
        // Advance only to the bytes actually verified above. An event is not an
        // upload acknowledgement, and a newer edit may already be on disk.
        baseline = version;
    }
}

fn report_watch_error(tx: &UnboundedSender<TransferJobResult>, job_id: &str, error: io::Error) {
    let _ = tx.unbounded_send(TransferJobResult {
        id: job_id.to_string(),
        event: TransferJobEvent::Finished(Err(format!("external editor watch failed: {error}"))),
    });
}

/// Streams on a worker; refuses to label a changing file as a stable version.
pub(in crate::features) fn read_external_editor_version(
    path: &Path,
    mut cancelled: impl FnMut() -> bool,
) -> io::Result<ExternalEditorFileVersion> {
    let mut file = fs::File::open(path)?;
    let before = file.metadata()?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut len = 0_u64;
    loop {
        if cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        len += read as u64;
    }
    let after = file.metadata()?;
    let current = fs::metadata(path)?;
    if len != before.len()
        || before.len() != after.len()
        || after.len() != current.len()
        || before.modified().ok() != after.modified().ok()
        || after.modified().ok() != current.modified().ok()
    {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "external edit changed while reading",
        ));
    }
    Ok(ExternalEditorFileVersion {
        len,
        modified: current.modified().ok(),
        digest: digest.finalize().into(),
    })
}

fn wait_for_stop(stop_rx: &mpsc::Receiver<()>, timeout: Duration) -> bool {
    match stop_rx.recv_timeout(timeout) {
        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => true,
        Err(mpsc::RecvTimeoutError::Timeout) => false,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use futures::channel::mpsc::unbounded;

    use super::{WatchTimings, read_external_editor_version, watch_external_editor_file};

    fn next_modification(
        rx: &mut futures::channel::mpsc::UnboundedReceiver<crate::models::TransferJobResult>,
    ) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(event) = rx.try_recv() {
                assert!(matches!(
                    event.event,
                    crate::models::TransferJobEvent::ExternalModified { .. }
                ));
                return;
            }
            assert!(Instant::now() < deadline, "missing edit notification");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn edits_before_watcher_start_and_immediate_same_length_saves_are_reported() {
        let root = nyaterm_core::test_support::TestTempDir::new("nyaterm-external-watch");
        fs::create_dir_all(root.path()).unwrap();
        let path = root.path().join("edit.txt");
        fs::write(&path, "value=1").unwrap();
        let baseline = read_external_editor_version(&path, || false).unwrap();
        // The editor can save before its completion event reaches the UI.
        fs::write(&path, "value=2").unwrap();
        let (tx, mut rx) = unbounded();
        let (stop_tx, stop_rx) = mpsc::channel();
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || {
            watch_external_editor_file(
                "job".into(),
                "/remote/edit".into(),
                None,
                worker_path,
                tx,
                stop_rx,
                WatchTimings {
                    interval: Duration::from_millis(5),
                    settle: Duration::from_millis(5),
                },
                baseline,
            )
        });
        next_modification(&mut rx);
        for value in 3..=5 {
            fs::write(&path, format!("value={value}")).unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(value))
                .unwrap();
            next_modification(&mut rx);
        }
        drop(stop_tx);
        worker.join().unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn timestamp_only_changes_do_not_emit_upload_notifications() {
        let root = nyaterm_core::test_support::TestTempDir::new("nyaterm-external-watch");
        fs::create_dir_all(root.path()).unwrap();
        let path = root.path().join("edit.txt");
        fs::write(&path, "unchanged").unwrap();
        let baseline = read_external_editor_version(&path, || false).unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(42))
            .unwrap();
        let current = read_external_editor_version(&path, || false).unwrap();
        assert_ne!(baseline.modified, current.modified);
        assert_eq!(baseline.digest, current.digest);
        let (tx, mut rx) = unbounded();
        let (stop_tx, stop_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            watch_external_editor_file(
                "job".into(),
                "/remote/edit".into(),
                None,
                path,
                tx,
                stop_rx,
                WatchTimings {
                    interval: Duration::from_millis(5),
                    settle: Duration::ZERO,
                },
                baseline,
            )
        });
        std::thread::sleep(Duration::from_millis(50));
        drop(stop_tx);
        worker.join().unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn version_read_rejects_concurrent_edits_and_honors_stop_between_chunks() {
        let root = nyaterm_core::test_support::TestTempDir::new("nyaterm-external-watch");
        fs::create_dir_all(root.path()).unwrap();
        let path = root.path().join("edit.txt");
        fs::write(&path, vec![b'a'; 128 * 1024]).unwrap();
        let mut calls = 0;
        let result = read_external_editor_version(&path, || {
            calls += 1;
            if calls == 2 {
                fs::write(&path, "new version").unwrap();
            }
            false
        });
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        assert_eq!(
            read_external_editor_version(&path, || true)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn watcher_stop_wakes_interval_wait_without_polling_delay() {
        let path = std::env::temp_dir().join(format!(
            "nyaterm-external-watch-{}-{}",
            std::process::id(),
            nyaterm_core::uuid()
        ));
        fs::write(&path, b"initial").expect("write fixture");
        let baseline = read_external_editor_version(&path, || false).unwrap();
        let (transfer_tx, _transfer_rx) = unbounded();
        let (stop_tx, stop_rx) = mpsc::channel();
        let worker_path = path.clone();
        let worker = std::thread::Builder::new()
            .name("nyaterm-external-watch-test".to_string())
            .spawn(move || {
                watch_external_editor_file(
                    "job".to_string(),
                    "/remote/file".to_string(),
                    None,
                    worker_path,
                    transfer_tx,
                    stop_rx,
                    WatchTimings {
                        interval: Duration::from_secs(30),
                        settle: Duration::ZERO,
                    },
                    baseline,
                );
            })
            .expect("spawn watcher");

        let started = Instant::now();
        drop(stop_tx);
        worker.join().expect("join watcher");
        assert!(started.elapsed() < Duration::from_secs(1));
        let _ = fs::remove_file(path);
    }
}
