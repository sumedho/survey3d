//! Desktop crash recovery. While there is unsaved work, a snapshot of the
//! project (as it would be if saved) is written to the app's data folder; a
//! clean exit, a save or discarding the changes removes it. After a crash the
//! next start finds the snapshot and offers it back.
//!
//! Each running app writes `recovery-<pid>.s3d` and holds a lock on
//! `recovery-<pid>.lock`. A snapshot whose lock can be taken belongs to an app
//! that is no longer running (the operating system drops locks when a program
//! stops, however it stops), so a second window open at the same time is never
//! mistaken for a crash.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::SystemTime;

/// A snapshot left by an app that stopped without saving.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub snapshot: PathBuf,
    /// The project file it was unsaved work on, if it had one.
    pub source: Option<String>,
    pub modified: SystemTime,
}

impl Found {
    /// Removes the snapshot and its companions.
    pub fn discard(&self) {
        for p in companions(&self.snapshot) {
            let _ = std::fs::remove_file(p);
        }
    }

    pub fn read(&self) -> std::io::Result<Vec<u8>> {
        std::fs::read(&self.snapshot)
    }
}

/// This app's snapshot slot.
pub struct Recovery {
    dir: PathBuf,
    snapshot: PathBuf,
    /// Held while running (see the module notes).
    _lock: File,
    writer: Option<JoinHandle<()>>,
}

/// `recovery-<id>.s3d` with its `.json` (source) and `.lock` companions.
fn companions(snapshot: &Path) -> [PathBuf; 3] {
    [
        snapshot.to_path_buf(),
        snapshot.with_extension("json"),
        snapshot.with_extension("lock"),
    ]
}

impl Recovery {
    /// The slot in `<app data>/recovery`, or `None` where there is no data
    /// folder or it can't be written.
    pub fn new(dir: PathBuf) -> Option<Self> {
        std::fs::create_dir_all(&dir).ok()?;
        let snapshot = dir.join(format!("recovery-{}.s3d", std::process::id()));
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(snapshot.with_extension("lock"))
            .ok()?;
        lock.try_lock().ok()?;
        let slot = Self {
            dir,
            snapshot,
            _lock: lock,
            writer: None,
        };
        slot.tidy();
        Some(slot)
    }

    /// Removes lock files of stopped sessions that left no snapshot (they
    /// stopped before any unsaved work, or after it was saved).
    fn tidy(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for p in entries.filter_map(Result::ok).map(|e| e.path()) {
            let stale = p.extension().is_some_and(|x| x == "lock")
                && !p.with_extension("s3d").exists()
                && p != self.snapshot.with_extension("lock")
                && File::options().write(true).open(&p).is_ok_and(|f| f.try_lock().is_ok());
            if stale {
                let _ = std::fs::remove_file(&p);
            }
        }
    }

    /// Snapshots left by apps no longer running, newest first.
    pub fn orphans(&self) -> Vec<Found> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut found: Vec<Found> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|x| x == "s3d")
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("recovery-"))
                    && *p != self.snapshot
            })
            .filter(|p| {
                // Taking the lock means its owner has stopped (released at once).
                OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .write(true)
                    .open(p.with_extension("lock"))
                    .is_ok_and(|f| f.try_lock().is_ok())
            })
            .filter_map(|p| {
                let modified = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
                let source = std::fs::read_to_string(p.with_extension("json"))
                    .ok()
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                    .and_then(|v| v.get("source").and_then(|s| s.as_str()).map(str::to_string));
                Some(Found {
                    snapshot: p,
                    source,
                    modified,
                })
            })
            .collect();
        found.sort_by_key(|f| std::cmp::Reverse(f.modified));
        found
    }

    /// Writes a snapshot in the background (written beside it and renamed into
    /// place, so a crash mid-write leaves the previous one). Skipped while the
    /// last write is still going.
    pub fn write(&mut self, bytes: Vec<u8>, source: Option<String>) {
        if self.writer.as_ref().is_some_and(|w| !w.is_finished()) {
            return;
        }
        let snapshot = self.snapshot.clone();
        self.writer = Some(std::thread::spawn(move || {
            let temp = snapshot.with_extension("s3d.writing");
            let meta = serde_json::json!({ "source": source }).to_string();
            let written = std::fs::write(&temp, &bytes)
                .and_then(|()| std::fs::write(snapshot.with_extension("json"), meta))
                .and_then(|()| std::fs::rename(&temp, &snapshot));
            if let Err(e) = written {
                log::warn!("crash-recovery snapshot failed: {e}");
                let _ = std::fs::remove_file(&temp);
            }
        }));
    }

    /// Removes this app's snapshot (the work was saved or discarded).
    pub fn clear(&mut self) {
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
        let _ = std::fs::remove_file(&self.snapshot);
        let _ = std::fs::remove_file(self.snapshot.with_extension("json"));
    }

    /// A clean exit: no snapshot, and the lock file goes too.
    pub fn close(mut self) {
        self.clear();
        let _ = std::fs::remove_file(self.snapshot.with_extension("lock"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("survey3d-recovery-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_crashed_session_is_found_and_a_running_one_is_not() {
        let d = dir("orphans");
        let mut me = Recovery::new(d.clone()).expect("slot");
        // Another session that crashed: snapshot present, lock not held.
        std::fs::create_dir_all(&d).expect("dir");
        let crashed = d.join("recovery-999999.s3d");
        std::fs::write(&crashed, b"project").expect("write");
        std::fs::write(crashed.with_extension("json"), r#"{"source":"/jobs/a.s3d"}"#).expect("write");
        // Another session still running: its lock is held.
        let running = d.join("recovery-999998.s3d");
        std::fs::write(&running, b"live").expect("write");
        let held = File::create(running.with_extension("lock")).expect("lock");
        held.try_lock().expect("hold");

        let found = me.orphans();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].snapshot, crashed);
        assert_eq!(found[0].source.as_deref(), Some("/jobs/a.s3d"));
        assert_eq!(found[0].read().expect("read"), b"project");
        found[0].discard();
        assert!(!crashed.exists() && !crashed.with_extension("json").exists());

        // Our own snapshot: written in the background, cleared on save.
        me.write(b"mine".to_vec(), None);
        if let Some(w) = me.writer.take() {
            w.join().expect("writer");
        }
        assert_eq!(std::fs::read(&me.snapshot).expect("own"), b"mine");
        assert!(me.orphans().is_empty(), "our own slot is not an orphan");
        me.clear();
        assert!(!me.snapshot.exists());
        drop(held);
        me.close();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_project_snapshot_comes_back_whole() {
        use survey_model::{ChangeSet, Entity, SurveyPoint};
        let d = dir("roundtrip");
        // A session with an unsaved point snapshots, then "crashes" (its slot
        // is dropped without closing: the lock goes, the snapshot stays).
        let mut project = crate::Project::new(&crate::settings::AppSettings::default()).expect("project");
        let point = Entity::Point(SurveyPoint {
            name: "UNSAVED".into(),
            position: glam::DVec3::new(1.0, 2.0, 3.0),
            height_known: true,
            code: String::new(),
            description: String::new(),
            layer: project.doc.current_layer(),
        });
        project
            .history
            .execute(Box::new(ChangeSet::new("add").insert_entity(point)), &mut project.doc);
        {
            let mut crashed = Recovery::new(d.clone()).expect("slot");
            crashed.write(project.snapshot().expect("snapshot"), Some("/jobs/site.s3d".into()));
            if let Some(w) = crashed.writer.take() {
                w.join().expect("writer");
            }
            // Simulate the lock of a stopped process going away (here: it is ours,
            // so release it by taking a fresh slot name).
            crashed.snapshot = d.join("recovery-1.s3d");
            std::fs::rename(d.join(format!("recovery-{}.s3d", std::process::id())), &crashed.snapshot)
                .expect("rename");
            std::fs::rename(
                d.join(format!("recovery-{}.json", std::process::id())),
                crashed.snapshot.with_extension("json"),
            )
            .expect("rename");
        }
        // A stray lock with no snapshot is tidied away by the next session.
        std::fs::write(d.join("recovery-2.lock"), b"").expect("stray");
        let next = Recovery::new(d.clone()).expect("next slot");
        assert!(!d.join("recovery-2.lock").exists());
        let found = next.orphans();
        assert_eq!(found.len(), 1, "{found:?}");
        let bytes = found[0].read().expect("read");
        let doc = survey_store::ProjectStore::from_bytes(&bytes)
            .expect("open")
            .load_document()
            .expect("load");
        assert!(doc.find_point_by_name("UNSAVED").is_some(), "unsaved work comes back");
        found[0].discard();
        next.close();
        let _ = std::fs::remove_dir_all(&d);
    }
}
