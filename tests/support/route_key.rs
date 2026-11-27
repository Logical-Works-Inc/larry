//! The key variable the tenant binding gives one state directory (larry#328):
//! `LARRY_KEY_<TAG>_<SUFFIX>`. This is a reference reading of the written
//! rule, not a call into the app, so the tests judge the shipped binding
//! instead of repeating it: the tag is the stored tenant text's first 16
//! characters, upper case, read from the state directory's own store
//! (larry#211: the tenant is stored, never derived from a path). A store that
//! is missing or mid-migration is driven to its end through the shipped route
//! entry point first; a store that still cannot be read is the ledger's own
//! typed error, source kept.

use std::path::Path;

use larry_demo::executor::ExecError;
use larry_demo::executor::routes::{self, RouteCliError};
use lgwks_deps::storefront::rusqlite::{Connection, OpenFlags};

/// Characters of the tenant text that make the tag (ADR 0085).
const TENANT_TAG_CHARS: usize = 16;

/// Reads the stored tenant's tag from the state directory's own store.
fn read_tag(state: &Path) -> Result<String, RouteCliError> {
    let connection =
        Connection::open_with_flags(state.join("executor.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|cause| RouteCliError::Exec(ExecError::Sql(cause)))?;
    let tenant: String = connection
        .query_row("SELECT tenant FROM dir_tenant WHERE id = 1", [], |row| {
            row.get(0)
        })
        .map_err(|cause| RouteCliError::Exec(ExecError::Sql(cause)))?;
    Ok(tenant.chars().take(TENANT_TAG_CHARS).collect())
}

/// The key variable name tenant `state` may name for `suffix`: the stored
/// tenant's tag, read from the store itself, so a moved directory keeps its
/// variables. A read that lands on a missing or half-migrated store drives the
/// list to its end first — racing opens migrate under the lock the list waits
/// on — and then reads again instead of judging a half-migrated store.
pub(crate) fn key_env(state: &Path, suffix: &str) -> Result<String, RouteCliError> {
    if read_tag(state).is_err() {
        let mut out = Vec::new();
        routes::run(
            |name| (name == "LARRY_STATE_DIR").then(|| state.to_string_lossy().into_owned()),
            &["list"],
            &mut out,
        )?;
    }
    read_tag(state).map(|tag| format!("LARRY_KEY_{}_{suffix}", tag.to_ascii_uppercase()))
}
