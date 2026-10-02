use std::path::PathBuf;
use std::sync::Mutex;

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::paths::{self, StorageProfile};
use crate::profile::restore;

pub struct AppState {
    pub profile: StorageProfile,
    pub data_dir: PathBuf,
    /// Startup failure is kept (not replaced with a fresh database) so the UI can report it.
    /// `None` only while a restored profile is being swapped in.
    db: Result<Mutex<Option<Database>>, AppError>,
}

impl AppState {
    pub fn open(profile: StorageProfile, data_dir: PathBuf) -> Self {
        // An interrupted profile restore is finished (or discarded) before anything opens.
        let db = paths::ensure_dir(&data_dir)
            .and_then(|()| restore::recover(&data_dir))
            .and_then(|_| Database::open(&data_dir.join(paths::DB_FILE_NAME)))
            .map(|db| Mutex::new(Some(db)));
        Self {
            profile,
            data_dir,
            db,
        }
    }

    pub fn failed(profile: StorageProfile, data_dir: PathBuf, err: AppError) -> Self {
        Self {
            profile,
            data_dir,
            db: Err(err),
        }
    }

    /// Path of the database file when storage opened successfully.
    pub fn database_path(&self) -> Option<PathBuf> {
        self.db
            .is_ok()
            .then(|| self.data_dir.join(paths::DB_FILE_NAME))
    }

    pub fn with_db<T>(&self, f: impl FnOnce(&mut Database) -> AppResult<T>) -> AppResult<T> {
        let db = self.db.as_ref().map_err(Clone::clone)?;
        let mut guard = db
            .lock()
            .map_err(|_| AppError::Internal("database lock poisoned".into()))?;
        let db = guard
            .as_mut()
            .ok_or_else(|| AppError::StorageUnavailable("a profile is being restored".into()))?;
        f(db)
    }

    /// Swap in the profile staged in `restore-staging/`. The live database is closed for
    /// the swap (so files can be renamed on every platform) and reopened afterwards. If
    /// the new profile cannot be opened, the previous one is put back.
    pub fn activate_staged_profile(&self) -> AppResult<()> {
        let db = self.db.as_ref().map_err(Clone::clone)?;
        let mut guard = db
            .lock()
            .map_err(|_| AppError::Internal("database lock poisoned".into()))?;
        drop(guard.take()); // closes the connection and checkpoints the WAL
        let path = self.data_dir.join(paths::DB_FILE_NAME);
        let activated = restore::activate(&self.data_dir).and_then(|()| Database::open(&path));
        match activated {
            Ok(new) => {
                *guard = Some(new);
                restore::finish(&self.data_dir)
            }
            Err(e) => {
                let back = restore::rollback(&self.data_dir).and_then(|()| Database::open(&path));
                match back {
                    Ok(old) => *guard = Some(old),
                    Err(e2) => eprintln!("MuDraft: couldn't reopen the previous profile: {e2}"),
                }
                Err(e)
            }
        }
    }
}
