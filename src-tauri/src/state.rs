use std::path::PathBuf;
use std::sync::Mutex;

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::paths::{self, StorageProfile};

pub struct AppState {
    pub profile: StorageProfile,
    pub data_dir: PathBuf,
    /// Startup failure is kept (not replaced with a fresh database) so the UI can report it.
    db: Result<Mutex<Database>, AppError>,
}

impl AppState {
    pub fn open(profile: StorageProfile, data_dir: PathBuf) -> Self {
        let db = paths::ensure_dir(&data_dir)
            .and_then(|()| Database::open(&data_dir.join(paths::DB_FILE_NAME)))
            .map(Mutex::new);
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
        f(&mut guard)
    }
}
