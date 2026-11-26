//! Tenant route table storage and lookup.

use super::Route;

/// A tenant's stored routes and which one is the default.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Table {
    /// The routes, ordered by id.
    routes: Vec<Route>,
    /// The id of the default route, when there is one.
    default: Option<String>,
}

impl Table {
    /// A table of `routes` whose default is `default`.
    pub(crate) fn new(routes: Vec<Route>, default: Option<String>) -> Table {
        Table { routes, default }
    }

    /// The routes, ordered by id.
    pub(crate) fn routes(&self) -> &[Route] {
        &self.routes
    }

    /// The id of the default route.
    pub(crate) fn default_id(&self) -> Option<&str> {
        self.default.as_deref()
    }

    /// The stored route `named` asks for, or the default when it names none.
    pub(crate) fn lookup(&self, named: Option<&str>) -> Option<&Route> {
        let wanted = named.or(self.default.as_deref())?;
        self.routes.iter().find(|route| route.id == wanted)
    }
}
