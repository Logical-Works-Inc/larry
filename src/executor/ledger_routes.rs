//! The route table of the executor ledger (larry#201): a tenant's routes, at
//! most 256, and the one default. Every statement names the tenant, so two
//! tenants sharing one store never read, replace or spend each other's rows.
//!
//! State: a route is absent or present; `put` makes it present or replaces its
//! fields (the default flag stays); `remove` makes it absent and, if it was the
//! default, leaves the tenant with none; `set_default` moves the one flag in a
//! single statement, so there is never a moment with two defaults or a default
//! naming a route that is gone.

use lgwks_deps::storefront::rusqlite::{self, Transaction, TransactionBehavior};

use super::Ledger;
use crate::executor::ExecError;
use crate::model::{Draft, Kind, MAX_ROUTES, Route, RouteError, Table};

/// Inserts a route, or replaces the fields of the one with this id, unless the
/// tenant holds the most routes it may and this id is new: that writes no row.
const UPSERT: &str = "INSERT INTO routes (tenant, id, kind, base_url, key_env, model, temperature, \
    request_bytes, residency, describe, is_default) \
    SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0 \
    WHERE EXISTS (SELECT 1 FROM routes WHERE tenant = ?1 AND id = ?2) \
    OR (SELECT COUNT(*) FROM routes WHERE tenant = ?1) < ?11 \
    ON CONFLICT (tenant, id) DO UPDATE SET kind = excluded.kind, base_url = excluded.base_url, \
    key_env = excluded.key_env, model = excluded.model, temperature = excluded.temperature, \
    request_bytes = excluded.request_bytes, residency = excluded.residency, \
    describe = excluded.describe";

/// One stored row as the engine returned it, before its fields are checked again.
struct Stored {
    /// The route id.
    id: String,
    /// The provider kind's name.
    kind: String,
    /// The base URL.
    base_url: String,
    /// The key variable's name.
    key_env: String,
    /// The model id.
    model: String,
    /// The pinned temperature, when there is one.
    temperature: Option<f64>,
    /// The request body bound.
    request_bytes: i64,
    /// The declared residency, when there is one.
    residency: Option<String>,
    /// The capability description.
    describe: String,
    /// 1 for the tenant's default route, else 0.
    is_default: i64,
}

impl Stored {
    /// The checked route this row holds, or the damage refusal naming its id.
    fn route(&self) -> Result<Route, RouteError> {
        let damaged = |cause: Option<RouteError>| RouteError::Damaged {
            id: self.id.clone(),
            cause: cause.map(Box::new),
        };
        let kind = Kind::parse(&self.kind).ok_or_else(|| damaged(None))?;
        let Ok(request_bytes) = usize::try_from(self.request_bytes) else {
            return Err(damaged(Some(RouteError::BadRequestBytes)));
        };
        Route::new(&Draft {
            id: &self.id,
            kind,
            base_url: &self.base_url,
            model: &self.model,
            key_env: &self.key_env,
            temperature: self.temperature,
            request_bytes: Some(request_bytes),
            residency: self.residency.as_deref(),
            describe: &self.describe,
        })
        .map_err(|refusal| damaged(Some(refusal)))
    }
}

impl Ledger {
    /// Stores `route` for `tenant`, replacing the fields of the route with its
    /// id, and makes it the tenant's default when `make_default`. A replacement
    /// keeps the default flag it had. One transaction, so a crash leaves the
    /// route stored or not, and the default moved or not.
    ///
    /// # Errors
    ///
    /// [`RouteError::TableFull`] (as [`ExecError::Route`]) when the tenant
    /// holds 256 routes and this id is new; [`ExecError::Sql`] otherwise.
    pub(crate) fn put_route(
        &self,
        tenant: &str,
        route: &Route,
        make_default: bool,
    ) -> Result<(), ExecError> {
        let Ok(bytes) = i64::try_from(route.request_bytes()) else {
            return Err(ExecError::Route(RouteError::BadRequestBytes));
        };
        let step = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
            .map_err(ExecError::Sql)?;
        let written = step
            .execute(
                UPSERT,
                rusqlite::params![
                    tenant,
                    route.id(),
                    route.kind().name(),
                    route.base_url(),
                    route.key_env(),
                    route.model(),
                    route.temperature(),
                    bytes,
                    route.residency(),
                    route.describe(),
                    i64::from(MAX_ROUTES)
                ],
            )
            .map_err(ExecError::Sql)?;
        if written == 0 {
            return Err(ExecError::Route(RouteError::TableFull {
                limit: usize::from(MAX_ROUTES),
            }));
        }
        if make_default {
            step.execute(
                "UPDATE routes SET is_default = (id = ?2) WHERE tenant = ?1",
                rusqlite::params![tenant, route.id()],
            )
            .map_err(ExecError::Sql)?;
        }
        step.commit().map_err(ExecError::Sql)
    }

    /// Every route `tenant` holds, ordered by id, and its default.
    ///
    /// # Errors
    ///
    /// [`RouteError::Damaged`] (as [`ExecError::Route`]) naming a stored row
    /// that fails validation (a key variable outside `LARRY_KEY_*` included) or
    /// a second default; [`ExecError::Sql`] otherwise.
    pub(crate) fn routes(&self, tenant: &str) -> Result<Table, ExecError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, kind, base_url, key_env, model, temperature, request_bytes, \
                 residency, describe, is_default FROM routes WHERE tenant = ?1 ORDER BY id LIMIT ?2",
            )
            .map_err(ExecError::Sql)?;
        let rows = statement
            .query_map(rusqlite::params![tenant, i64::from(MAX_ROUTES)], |row| {
                Ok(Stored {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    base_url: row.get(2)?,
                    key_env: row.get(3)?,
                    model: row.get(4)?,
                    temperature: row.get(5)?,
                    request_bytes: row.get(6)?,
                    residency: row.get(7)?,
                    describe: row.get(8)?,
                    is_default: row.get(9)?,
                })
            })
            .map_err(ExecError::Sql)?;
        let mut routes = Vec::new();
        let mut default: Option<String> = None;
        for row in rows {
            let stored = row.map_err(ExecError::Sql)?;
            let route = stored.route().map_err(ExecError::Route)?;
            match stored.is_default {
                0 => {}
                1 if default.is_none() => default = Some(stored.id),
                _ => {
                    return Err(ExecError::Route(RouteError::Damaged {
                        id: stored.id,
                        cause: None,
                    }));
                }
            }
            routes.push(route);
        }
        Ok(Table::new(routes, default))
    }

    /// Removes `tenant`'s route `id`. A removed default leaves the tenant with none.
    ///
    /// # Errors
    ///
    /// [`RouteError::Unknown`] (as [`ExecError::Route`]) when the tenant has no
    /// such route; [`ExecError::Sql`] otherwise.
    pub(crate) fn remove_route(&self, tenant: &str, id: &str) -> Result<(), ExecError> {
        let removed = self
            .conn
            .execute(
                "DELETE FROM routes WHERE tenant = ?1 AND id = ?2",
                rusqlite::params![tenant, id],
            )
            .map_err(ExecError::Sql)?;
        if removed == 0 {
            return Err(ExecError::Route(RouteError::Unknown { id: id.to_owned() }));
        }
        Ok(())
    }

    /// Makes `id` the one default route of `tenant`, in one statement that
    /// clears the old default.
    ///
    /// # Errors
    ///
    /// [`RouteError::Unknown`] (as [`ExecError::Route`]) when the tenant has no
    /// such route, which changes nothing; [`ExecError::Sql`] otherwise.
    pub(crate) fn set_default_route(&self, tenant: &str, id: &str) -> Result<(), ExecError> {
        let moved = self
            .conn
            .execute(
                "UPDATE routes SET is_default = (id = ?2) WHERE tenant = ?1 \
                 AND EXISTS (SELECT 1 FROM routes WHERE tenant = ?1 AND id = ?2)",
                rusqlite::params![tenant, id],
            )
            .map_err(ExecError::Sql)?;
        if moved == 0 {
            return Err(ExecError::Route(RouteError::Unknown { id: id.to_owned() }));
        }
        Ok(())
    }
}

/// The store round trip, the tenant wall and the damage refusal, through `Ledger` only.
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{Draft, Kind, Ledger, Route, RouteError};
    use crate::executor::ExecError;

    /// Per-process counter that gives each test its own state directory.
    static DIRECTORIES: AtomicU64 = AtomicU64::new(0);

    /// A route over `id` with `model`.
    fn route(id: &str, model: &str) -> Result<Route, RouteError> {
        Route::new(&Draft {
            id,
            kind: Kind::OpenAiChat,
            base_url: "http://gateway.test:9/v1",
            model,
            key_env: "LARRY_KEY_TEST",
            temperature: Some(0.5),
            request_bytes: None,
            residency: Some("CA"),
            describe: "a test route",
        })
    }

    /// A fresh ledger in its own directory, and the directory.
    fn open() -> Result<(Ledger, std::path::PathBuf), Box<dyn std::error::Error>> {
        let index = DIRECTORIES.fetch_add(1, Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("larry-routes-{}-{index}", std::process::id()));
        drop(std::fs::remove_dir_all(&directory));
        Ok((Ledger::open(&directory)?, directory))
    }

    /// Routes put, read back, defaulted, replaced and removed round trip; two
    /// tenants on one id never see each other's row; a row edited into damage is refused.
    #[test]
    fn route_store_round_trips_through_the_ledger() -> Result<(), Box<dyn std::error::Error>> {
        let (ledger, directory) = open()?;
        ledger.put_route("tenant-a", &route("alpha", "model-a1")?, true)?;
        ledger.put_route("tenant-a", &route("beta", "model-a2")?, false)?;
        ledger.put_route("tenant-b", &route("alpha", "model-b1")?, false)?;
        let table = ledger.routes("tenant-a")?;
        let ids: Vec<&str> = table.routes().iter().map(Route::id).collect();
        assert_eq!(ids, ["alpha", "beta"], "ordered by id, only this tenant's");
        assert_eq!(table.default_id(), Some("alpha"), "the first default");
        assert_eq!(
            ledger.routes("tenant-b")?.default_id(),
            None,
            "another tenant's default is its own"
        );

        ledger.set_default_route("tenant-a", "beta")?;
        assert_eq!(
            ledger.routes("tenant-a")?.default_id(),
            Some("beta"),
            "the default moved"
        );
        ledger.put_route("tenant-a", &route("alpha", "model-a3")?, false)?;
        let replaced = ledger.routes("tenant-a")?;
        assert_eq!(
            replaced.default_id(),
            Some("beta"),
            "a replacement without --default keeps the default where it was"
        );
        assert_eq!(
            replaced.lookup(Some("alpha")).map(Route::model),
            Some("model-a3"),
            "the replacement changed the fields"
        );
        assert_eq!(
            ledger
                .routes("tenant-b")?
                .lookup(Some("alpha"))
                .map(Route::model),
            Some("model-b1"),
            "tenant b's alpha is untouched by tenant a's"
        );

        assert!(
            matches!(
                ledger.set_default_route("tenant-a", "nope"),
                Err(ExecError::Route(RouteError::Unknown { .. }))
            ),
            "an unknown default is refused"
        );
        assert_eq!(
            ledger.routes("tenant-a")?.default_id(),
            Some("beta"),
            "a refused default changes nothing"
        );
        ledger.remove_route("tenant-a", "beta")?;
        assert_eq!(
            ledger.routes("tenant-a")?.default_id(),
            None,
            "a removed default leaves none"
        );
        assert!(
            matches!(
                ledger.remove_route("tenant-a", "beta"),
                Err(ExecError::Route(RouteError::Unknown { .. }))
            ),
            "removing twice is refused"
        );

        ledger.conn.execute(
            "UPDATE routes SET key_env = 'GH_TOKEN' WHERE tenant = 'tenant-b'",
            [],
        )?;
        assert!(
            matches!(
                ledger.routes("tenant-b"),
                Err(ExecError::Route(RouteError::Damaged { cause: Some(ref inner), .. }))
                    if **inner == RouteError::KeyEnvOutsideNamespace
            ),
            "a row naming a variable outside LARRY_KEY_ is damaged, not read"
        );
        ledger.conn.execute(
            "UPDATE routes SET key_env = 'LARRY_KEY_TEST', kind = 'bogus' WHERE tenant = 'tenant-b'",
            [],
        )?;
        assert!(
            matches!(
                ledger.routes("tenant-b"),
                Err(ExecError::Route(RouteError::Damaged { .. }))
            ),
            "a damaged row is refused, never skipped"
        );
        assert!(
            ledger.routes("tenant-a").is_ok(),
            "tenant a still reads: the damage is tenant b's"
        );
        drop(std::fs::remove_dir_all(&directory));
        Ok(())
    }
}
