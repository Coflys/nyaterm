use super::super::duplicate::{DuplicateChoice, TransferDuplicateManager};
use super::*;

#[test]
fn skipped_download_event_is_not_completed_and_does_not_register_a_target() {
    for is_directory in [false, true] {
        let id = uuid::Uuid::new_v4().to_string();
        let event = build_skipped_download_event(
            "session-1",
            "/remote/app.log",
            "/local/app.log",
            Some(id.clone()),
            is_directory,
        );
        assert_eq!(event.id, id);
        assert_eq!(event.status, "skipped");
        assert_eq!(event.direction, "download");
        assert_eq!(event.kind, if is_directory { "directory" } else { "file" });
        assert_eq!(event.file_name, "app.log");
        assert_eq!(event.size, 0);
        assert_eq!(event.bytes_transferred, 0);
        assert_eq!(event.total_size, 0);
        assert!(event.error_msg.is_none());
        assert!(find_transfer(&id).is_none());
        assert!(find_transfer_target(&id).is_none());
    }
}

#[tokio::test]
async fn preparing_download_cancellation_ends_hidden_and_displayed_prompts() {
    for shown in [false, true] {
        let id = uuid::Uuid::new_v4().to_string();
        let preparing = create_child_file_transfer_controller(
            Some(id.clone()),
            "session",
            "app.log".to_string(),
            "/remote/app.log",
            "",
            "download",
            None,
        );
        let retained = RetainedTransfer::new(preparing.clone());
        let manager = Arc::new(TransferDuplicateManager::new());
        let (reply, displayed) = manager.register("cancelled".to_string()).await;
        if shown {
            assert!(manager.mark_displayed("cancelled").await);
        }
        let waiting = {
            let manager = manager.clone();
            let controller = preparing.clone();
            tokio::spawn(async move {
                manager
                    .wait_for_response(
                        "cancelled",
                        reply,
                        displayed,
                        Some(&controller),
                        std::time::Duration::from_secs(120),
                        std::time::Duration::from_secs(600),
                    )
                    .await
            })
        };
        tokio::task::yield_now().await;
        preparing.cancel();
        assert!(matches!(
            waiting.await.unwrap(),
            Err(AppError::Cancelled(_))
        ));
        assert!(!manager.mark_displayed("cancelled").await);
        assert!(
            !manager
                .respond("cancelled", DuplicateChoice::Overwrite)
                .await
        );
        assert!(matches!(
            retained.ready().await,
            Err(AppError::Cancelled(_))
        ));
        drop(retained);
        assert!(find_transfer(&id).is_none());
        assert!(find_transfer_target(&id).is_none());
    }
}

#[tokio::test]
async fn preparing_download_cancellation_survives_the_handoff_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.log").to_string_lossy().to_string();
    std::fs::write(&path, b"previous download").unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let preparing = create_child_file_transfer_controller(
        Some(id.clone()),
        "session",
        "app.log".to_string(),
        "/remote/app.log",
        "",
        "download",
        None,
    );
    let retained = RetainedTransfer::new(preparing.clone());
    let manager = TransferDuplicateManager::new();
    let (reply, displayed) = manager.register("answered".to_string()).await;
    assert!(
        manager
            .respond("answered", DuplicateChoice::Overwrite)
            .await
    );
    assert_eq!(
        manager
            .wait_for_response(
                "answered",
                reply,
                displayed,
                Some(&preparing),
                std::time::Duration::from_secs(120),
                std::time::Duration::from_secs(600)
            )
            .await
            .unwrap(),
        DuplicateChoice::Overwrite
    );
    preparing.cancel();
    let actual = create_child_file_transfer_controller(
        Some(id.clone()),
        "session",
        "app.log".to_string(),
        "/remote/app.log",
        &path,
        "download",
        None,
    );
    register_transfer(actual.clone());
    assert!(matches!(
        wait_for_transfer_chain(&actual, None).await,
        Err(AppError::Cancelled(_))
    ));
    unregister_transfer(&id);
    assert!(matches!(
        retained.ready().await,
        Err(AppError::Cancelled(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"previous download");
    drop(retained);
    assert!(find_transfer(&id).is_none());
}

#[test]
fn download_rename_preserves_file_extension_and_full_directory_name() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("app.log").to_string_lossy().to_string();
    std::fs::write(&file, b"first server").unwrap();
    std::fs::write(dir.path().join("app(1).log"), b"previous download").unwrap();
    assert_eq!(
        generate_unique_local_path(&file, false).unwrap(),
        dir.path().join("app(2).log").to_string_lossy()
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"first server");

    let folder = dir.path().join("logs.2026").to_string_lossy().to_string();
    std::fs::create_dir(&folder).unwrap();
    assert_eq!(
        generate_unique_local_path(&folder, true).unwrap(),
        dir.path().join("logs.2026(1)").to_string_lossy()
    );
    assert_eq!(local_rename_candidate(".env", false, 1), ".env(1)");
    assert_eq!(local_rename_candidate("README", false, 1), "README(1)");
}

#[test]
fn download_rename_exhaustion_never_returns_original_path() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("app.log").to_string_lossy().to_string();
    std::fs::write(&file, b"original").unwrap();
    for index in 1..=999 {
        std::fs::write(local_rename_candidate(&file, false, index), b"").unwrap();
    }
    assert!(generate_unique_local_path(&file, false).is_err());
    assert!(reserve_renamed_local_download_target(&file, false).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"original");
}

#[test]
fn download_reservation_protects_paths_before_files_are_created_and_releases_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("app.log").to_string_lossy().to_string();
    let first = reserve_local_download_target(&file, false, false)
        .unwrap()
        .unwrap();
    assert!(!Path::new(&file).exists());
    assert!(
        reserve_local_download_target(&file, false, false)
            .unwrap()
            .is_none()
    );
    assert!(reserve_local_download_target(&file, false, true).is_err());
    let second = reserve_renamed_local_download_target(&file, false).unwrap();
    let third = reserve_renamed_local_download_target(&file, false).unwrap();
    assert_eq!(second.path, dir.path().join("app(1).log").to_string_lossy());
    assert_eq!(third.path, dir.path().join("app(2).log").to_string_lossy());
    drop(first);
    assert!(
        reserve_local_download_target(&file, false, false)
            .unwrap()
            .is_some()
    );
}

#[test]
fn download_reservation_rejects_overlapping_directory_transfers() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("logs").to_string_lossy().to_string();
    let child = dir
        .path()
        .join("logs/app.log")
        .to_string_lossy()
        .to_string();
    let target = reserve_local_download_target(&folder, true, false)
        .unwrap()
        .unwrap();
    assert!(reserve_local_download_target(&child, false, true).is_err());
    drop(target);
    let _child_target = reserve_local_download_target(&child, false, false)
        .unwrap()
        .unwrap();
    assert!(reserve_local_download_target(&folder, true, true).is_err());
}

#[test]
fn download_reservation_keeps_existing_file_until_overwrite_is_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("app.log").to_string_lossy().to_string();
    std::fs::write(&file, b"original").unwrap();
    assert!(
        reserve_local_download_target(&file, false, false)
            .unwrap()
            .is_none()
    );
    let _target = reserve_local_download_target(&file, false, true)
        .unwrap()
        .unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"original");
}

mod staged_download_tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    fn controller(path: &Path) -> Arc<TransferController> {
        create_child_file_transfer_controller(
            None,
            "session",
            "app.log".to_string(),
            "/remote/app.log",
            &path.to_string_lossy(),
            "download",
            None,
        )
    }

    async fn stage(path: &Path, contents: &[u8]) -> (StagedLocalDownload, PathBuf) {
        let mut download = StagedLocalDownload::new(&path.to_string_lossy())
            .await
            .unwrap();
        let staged_path = download.temp_path.to_path_buf();
        assert_eq!(staged_path.parent(), path.parent());
        assert_ne!(staged_path, path);
        download.file_mut().write_all(contents).await.unwrap();
        download.file_mut().flush().await.unwrap();
        assert_eq!(std::fs::read(&staged_path).unwrap(), contents);
        (download, staged_path)
    }

    fn assert_entries(path: &Path, expected: &[&str]) {
        let mut actual: Vec<_> = std::fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        actual.sort();
        let mut expected: Vec<_> = expected.iter().map(|name| name.to_string()).collect();
        expected.sort();
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn overwrite_cancel_after_partial_write_preserves_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        std::fs::write(&path, b"original server log").unwrap();
        let controller = controller(&path);
        let (download, staged_path) = stage(&path, b"partial new server log").await;
        assert_eq!(std::fs::read(&path).unwrap(), b"original server log");

        controller.cancel();
        assert!(matches!(
            download.commit(&controller, None, None).await,
            Err(AppError::Cancelled(_))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"original server log");
        assert!(!staged_path.exists());
        assert_entries(dir.path(), &["app.log"]);
    }

    #[tokio::test]
    async fn new_file_cancel_after_partial_write_leaves_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        let controller = controller(&path);
        let (download, _) = stage(&path, b"partial log").await;
        assert!(!path.exists());

        controller.cancel();
        assert!(matches!(
            download.commit(&controller, None, None).await,
            Err(AppError::Cancelled(_))
        ));
        assert_entries(dir.path(), &[]);
    }

    #[tokio::test]
    async fn successful_commit_updates_only_the_final_path_including_empty_files() {
        for exists in [false, true] {
            for contents in [&b"complete new log"[..], &b""[..]] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("app.log");
                if exists {
                    std::fs::write(&path, b"old longer server log").unwrap();
                }
                let controller = controller(&path);
                let (download, staged_path) = stage(&path, contents).await;
                assert_eq!(path.exists(), exists);
                if exists {
                    assert_eq!(std::fs::read(&path).unwrap(), b"old longer server log");
                }

                download.commit(&controller, None, None).await.unwrap();
                assert_eq!(std::fs::read(&path).unwrap(), contents);
                assert!(!staged_path.exists());
                assert_entries(dir.path(), &["app.log"]);
            }
        }
    }

    #[tokio::test]
    async fn dropping_partial_download_cleans_only_its_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        std::fs::write(&path, b"original").unwrap();
        let (download, _) = stage(&path, b"partial new log").await;

        // 模拟远端读取/完整性校验失败，离开作用域时不执行提交。
        drop(download);
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_entries(dir.path(), &["app.log"]);
    }

    #[tokio::test]
    async fn failed_commit_keeps_existing_destination_and_cleans_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        // 文件不能替换包含原有文件的目录；不依赖运行用户权限即可稳定触发失败。
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("keep.log"), b"original").unwrap();
        let controller = controller(&path);
        let (download, staged_path) = stage(&path, b"new log").await;

        assert!(
            matches!(download.commit(&controller, None, None).await, Err(AppError::Channel(message)) if message.starts_with("Failed to commit download"))
        );
        assert_eq!(std::fs::read(path.join("keep.log")).unwrap(), b"original");
        assert!(!staged_path.exists());
        assert_entries(dir.path(), &["app.log"]);
    }

    #[tokio::test]
    async fn directory_cancel_preserves_unfinished_child_and_keeps_committed_children() {
        // SFTP 的 worker 直接使用目录 controller，SCP 的子任务还需检查父 controller。
        for use_child_controller in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("app.log");
            let completed = dir.path().join("completed.log");
            std::fs::write(&path, b"old app log").unwrap();
            std::fs::write(dir.path().join("keep.log"), b"untouched").unwrap();
            std::fs::write(&completed, b"old completed log").unwrap();
            let parent = create_directory_transfer_controller(
                None,
                "session",
                "logs".to_string(),
                "/remote/logs",
                &dir.path().to_string_lossy(),
                "download",
                2,
                100,
            );
            let child = controller(&path);
            let (first, _) = stage(&completed, b"fully downloaded log").await;
            first.commit(&parent, None, None).await.unwrap();
            let (download, _) = stage(&path, b"partial app log").await;

            parent.cancel();
            let result = if use_child_controller {
                download.commit(&child, Some(&parent), None).await
            } else {
                download.commit(&parent, None, None).await
            };
            assert!(matches!(result, Err(AppError::Cancelled(_))));
            assert_eq!(std::fs::read(&path).unwrap(), b"old app log");
            assert_eq!(
                std::fs::read(dir.path().join("keep.log")).unwrap(),
                b"untouched"
            );
            assert_eq!(std::fs::read(&completed).unwrap(), b"fully downloaded log");
            assert_entries(dir.path(), &["app.log", "completed.log", "keep.log"]);
        }
    }

    #[tokio::test]
    async fn aborting_directory_worker_drops_staging_file_without_touching_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        std::fs::write(&path, b"original").unwrap();
        let worker_path = path.clone();
        let (ready, started) = tokio::sync::oneshot::channel();
        let mut workers = tokio::task::JoinSet::new();
        workers.spawn(async move {
            let (download, staged_path) = stage(&worker_path, b"partial log").await;
            ready.send(staged_path).unwrap();
            std::future::pending::<()>().await;
            drop(download);
        });
        let staged_path = started.await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"original");

        workers.abort_all();
        assert!(
            workers
                .join_next()
                .await
                .unwrap()
                .unwrap_err()
                .is_cancelled()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert!(!staged_path.exists());
        assert_entries(dir.path(), &["app.log"]);
    }

    #[tokio::test]
    async fn successful_commit_preserves_requested_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        let controller = controller(&path);
        let (download, _) = stage(&path, b"complete log").await;
        let modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);

        download
            .commit(&controller, None, Some(modified))
            .await
            .unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
        assert_entries(dir.path(), &["app.log"]);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn locked_target_commit_failure_preserves_original_file() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        std::fs::write(&path, b"original").unwrap();
        let controller = controller(&path);
        let (download, staged_path) = stage(&path, b"new complete log").await;
        let locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();

        assert!(
            matches!(download.commit(&controller, None, None).await, Err(AppError::Channel(message)) if message.starts_with("Failed to commit download"))
        );
        drop(locked);
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert!(!staged_path.exists());
        assert_entries(dir.path(), &["app.log"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn commit_keeps_existing_permissions_and_normal_new_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        let reference = dir.path().join("reference.log");
        std::fs::File::create(&reference).unwrap();
        let ordinary_mode = std::fs::metadata(&reference).unwrap().permissions().mode() & 0o777;
        let controller = controller(&path);
        let (download, _) = stage(&path, b"new log").await;
        download.commit(&controller, None, None).await.unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            ordinary_mode
        );

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        let (download, _) = stage(&path, b"replacement").await;
        download.commit(&controller, None, None).await.unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
}

#[cfg(windows)]
#[test]
fn download_reservation_recognizes_windows_case_aliases() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("App.log").to_string_lossy().to_string();
    let alias = dir.path().join("app.log").to_string_lossy().to_string();
    let _target = reserve_local_download_target(&file, false, false)
        .unwrap()
        .unwrap();
    assert!(
        reserve_local_download_target(&alias, false, false)
            .unwrap()
            .is_none()
    );
}
