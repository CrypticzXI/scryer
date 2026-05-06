use std::os::raw::{c_char, c_int};
use std::sync::OnceLock;

use libsqlite3_sys::{SQLITE_OK, sqlite3, sqlite3_api_routines, sqlite3_auto_extension};
use scryer_application::{AppError, AppResult};

unsafe extern "C" {
    fn sqlite3_spellfix_init(
        db: *mut sqlite3,
        pz_err_msg: *mut *mut c_char,
        p_api: *const sqlite3_api_routines,
    ) -> c_int;
}

static SPELLFIX_AUTO_EXTENSION: OnceLock<Result<(), String>> = OnceLock::new();

pub fn register_spellfix_auto_extension() -> AppResult<()> {
    let result = SPELLFIX_AUTO_EXTENSION.get_or_init(|| {
        let rc = unsafe { sqlite3_auto_extension(Some(sqlite3_spellfix_init)) };
        if rc == SQLITE_OK {
            Ok(())
        } else {
            Err(format!(
                "sqlite3_auto_extension(spellfix1) failed with code {rc}"
            ))
        }
    });

    match result {
        Ok(()) => Ok(()),
        Err(message) => Err(AppError::Repository(message.clone())),
    }
}
