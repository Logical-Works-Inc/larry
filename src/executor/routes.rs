//! `larry route` and the route a task resolves to (larry#201). A route is data a
//! tenant owns in the state directory's store: `larry route add` stores one,
//! `list` shows them, `remove` and `default` change the table, and `resolve`
//! shows which route a task naming an id (or none) would use. The key is never
//! an argument and never stored: a route names the `LARRY_KEY_*` variable that
//! holds it, and the variable is read only when a task reaches the model.

use std::fmt;
use std::io::{self, Write};
use std::path::Path;

use lgwks_std::json::{Value, json};

use super::{ExecError, Ledger, digest_hex, identity, state_dir};
use crate::model::{Draft, Kind, Route, RouteError, Table, env_route, is_route_id, resolve};

/// The synopsis every usage refusal ends with.
const USAGE: &str = "usage: larry route add <id> --url <url> --model <model> --key-env <LARRY_KEY_NAME> --residency <CC> [--temperature <t>] [--request-bytes <n>] [--describe <text>] [--default] | list [--json] | remove <id> | default <id> | resolve [<id>]";

/// Why `larry route` did not finish.
#[derive(Debug)]
#[non_exhaustive]
pub enum RouteCliError {
    /// The arguments are not a `larry route` command.
    Usage(&'static str),
    /// An argument is not UTF-8 text.
    NotUtf8,
    /// `route add` needs a flag it was not given.
    Missing {
        /// The flag.
        flag: &'static str,
    },
    /// A numeric flag holds something that is not a number.
    BadNumber {
        /// The flag.
        flag: &'static str,
        /// Why the text is not a number, in the parser's words.
        reason: String,
    },
    /// A route, a route id or a resolution was refused.
    Route(RouteError),
    /// The state directory or the store failed.
    Exec(ExecError),
    /// Standard output could not be written.
    Output(io::Error),
}

impl fmt::Display for RouteCliError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Usage(reason) => write!(out, "route: {reason}; {USAGE}"),
            Self::NotUtf8 => out.write_str("route: every argument must be UTF-8 text"),
            Self::Missing { flag } => write!(out, "route add needs {flag}"),
            Self::BadNumber { flag, ref reason } => {
                write!(out, "route {flag} is not a number: {reason}")
            }
            Self::Route(ref cause) => write!(out, "route: {cause}"),
            Self::Exec(ref cause) => write!(out, "route: {cause}"),
            Self::Output(ref cause) => write!(out, "route output unwritable: {cause}"),
        }
    }
}

impl std::error::Error for RouteCliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Route(ref cause) => Some(cause),
            Self::Exec(ref cause) => Some(cause),
            Self::Output(ref cause) => Some(cause),
            Self::Usage(_) | Self::NotUtf8 | Self::Missing { .. } | Self::BadNumber { .. } => None,
        }
    }
}

/// A refusal of the route table as the command's own, any other fault as the store's.
fn from_exec(error: ExecError) -> RouteCliError {
    match error {
        ExecError::Route(cause) => RouteCliError::Route(cause),
        other => RouteCliError::Exec(other),
    }
}

/// The tenant a state directory is, as the text every route row is keyed by.
fn tenant_text(state: &Path) -> String {
    digest_hex(identity::tenant_of(state).as_bytes())
}

/// The route a task naming `named` (or none) sends its model calls to under the
/// state directory `state`: the named route, else the tenant's default, else
/// the environment route, else a refusal. The environment is parsed only when
/// no stored route answers, so a typo in a knob nobody uses fails nothing.
///
/// # Errors
///
/// [`ExecError::Route`] for an unknown name, no route at all or a damaged row;
/// [`ExecError::ModelConfig`] for an environment route that cannot be read.
pub(super) fn resolve_for(
    ledger: &Ledger,
    state: &Path,
    named: Option<&str>,
    read: &impl Fn(&str) -> Option<String>,
) -> Result<Route, ExecError> {
    let table = ledger.routes(&tenant_text(state))?;
    let environment = if table.lookup(named).is_some() {
        None
    } else {
        env_route(read).map_err(ExecError::ModelConfig)?
    };
    resolve(&table, named, environment).map_err(ExecError::Route)
}

/// The flags of `route add`, each still text.
#[derive(Default)]
struct Flags<'arg> {
    /// `--url`.
    url: Option<&'arg str>,
    /// `--model`.
    model: Option<&'arg str>,
    /// `--key-env`.
    key_env: Option<&'arg str>,
    /// `--residency`.
    residency: Option<&'arg str>,
    /// `--temperature`.
    temperature: Option<&'arg str>,
    /// `--request-bytes`.
    request_bytes: Option<&'arg str>,
    /// `--describe`.
    describe: Option<&'arg str>,
    /// `--default`.
    make_default: bool,
}

/// Reads `flags` once, refusing an unknown flag, a flag without its value and a repeat.
fn parse_flags<'arg>(flags: &[&'arg str]) -> Result<Flags<'arg>, RouteCliError> {
    let mut parsed = Flags::default();
    let mut rest = flags.iter().copied();
    while let Some(flag) = rest.next() {
        if flag == "--default" {
            if parsed.make_default {
                return Err(RouteCliError::Usage("a flag was given twice"));
            }
            parsed.make_default = true;
            continue;
        }
        let slot = match flag {
            "--url" => &mut parsed.url,
            "--model" => &mut parsed.model,
            "--key-env" => &mut parsed.key_env,
            "--residency" => &mut parsed.residency,
            "--temperature" => &mut parsed.temperature,
            "--request-bytes" => &mut parsed.request_bytes,
            "--describe" => &mut parsed.describe,
            _ => return Err(RouteCliError::Usage("unknown flag")),
        };
        let value = rest
            .next()
            .ok_or(RouteCliError::Usage("a flag needs its value"))?;
        if slot.replace(value).is_some() {
            return Err(RouteCliError::Usage("a flag was given twice"));
        }
    }
    Ok(parsed)
}

/// The value of a flag `route add` requires.
fn required<'arg>(
    value: Option<&'arg str>,
    flag: &'static str,
) -> Result<&'arg str, RouteCliError> {
    value.ok_or(RouteCliError::Missing { flag })
}

/// `text` as a number of type `Number`, or the flag's refusal.
fn number<Number: std::str::FromStr>(
    text: Option<&str>,
    flag: &'static str,
) -> Result<Option<Number>, RouteCliError>
where
    Number::Err: fmt::Display,
{
    text.map(|digits| {
        digits
            .parse::<Number>()
            .map_err(|cause| RouteCliError::BadNumber {
                flag,
                reason: cause.to_string(),
            })
    })
    .transpose()
}

/// Writes one line to `out` and flushes it.
fn say(out: &mut impl Write, line: &dyn fmt::Display) -> Result<(), RouteCliError> {
    writeln!(out, "{line}")
        .and_then(|()| out.flush())
        .map_err(RouteCliError::Output)
}

/// A route id from the command line, checked before the store is asked about it.
fn id_arg(id: &str) -> Result<&str, RouteCliError> {
    if is_route_id(id) {
        Ok(id)
    } else {
        Err(RouteCliError::Route(RouteError::BadId))
    }
}

/// The store and the tenant it belongs to, for the state directory `read` names.
fn open(read: &impl Fn(&str) -> Option<String>) -> Result<(Ledger, String), RouteCliError> {
    let state = state_dir(read).map_err(from_exec)?;
    let ledger = Ledger::open(&state).map_err(from_exec)?;
    Ok((ledger, tenant_text(&state)))
}

/// `larry route add <id> ...`: checks every field, then stores the route.
fn add(
    read: &impl Fn(&str) -> Option<String>,
    id: &str,
    flags: &[&str],
    out: &mut impl Write,
) -> Result<(), RouteCliError> {
    let parsed = parse_flags(flags)?;
    // A route added without `--describe` has the empty description.
    let mut describe = "";
    if let Some(text) = parsed.describe {
        describe = text;
    }
    let route = Route::new(&Draft {
        id,
        kind: Kind::OpenAiChat,
        base_url: required(parsed.url, "--url")?,
        model: required(parsed.model, "--model")?,
        key_env: required(parsed.key_env, "--key-env")?,
        temperature: number(parsed.temperature, "--temperature")?,
        request_bytes: number(parsed.request_bytes, "--request-bytes")?,
        residency: Some(required(parsed.residency, "--residency")?),
        describe,
    })
    .map_err(RouteCliError::Route)?;
    let (ledger, tenant) = open(read)?;
    ledger
        .put_route(&tenant, &route, parsed.make_default)
        .map_err(from_exec)?;
    say(
        out,
        &json!({ "route": route.id(), "saved": true, "made_default": parsed.make_default }),
    )
}

/// One route as the JSON listing prints it.
fn listed(route: &Route, table: &Table) -> Value {
    json!({
        "id": route.id(),
        "kind": route.kind().name(),
        "url": route.base_url(),
        "host": route.host(),
        "key_env": route.key_env(),
        "model": route.model(),
        "temperature": route.temperature(),
        "request_bytes": route.request_bytes(),
        "residency": route.residency(),
        "describe": route.describe(),
        "default": table.default_id() == Some(route.id()),
    })
}

/// `larry route list [--json]`: the tenant's routes, ordered by id.
fn list(
    read: &impl Fn(&str) -> Option<String>,
    as_json: bool,
    out: &mut impl Write,
) -> Result<(), RouteCliError> {
    let (ledger, tenant) = open(read)?;
    let table = ledger.routes(&tenant).map_err(from_exec)?;
    if as_json {
        let routes: Vec<Value> = table
            .routes()
            .iter()
            .map(|route| listed(route, &table))
            .collect();
        return say(
            out,
            &json!({ "default": table.default_id(), "routes": routes }),
        );
    }
    for route in table.routes() {
        // A route that declares no residency prints a dash in its column.
        let mut residency = "-";
        if let Some(code) = route.residency() {
            residency = code;
        }
        let marker = if table.default_id() == Some(route.id()) {
            "\tdefault"
        } else {
            ""
        };
        say(
            out,
            &format_args!(
                "{}\t{}\t{}\t{residency}{marker}",
                route.id(),
                route.model(),
                route.host()
            ),
        )?;
    }
    Ok(())
}

/// `larry route remove <id>`: deletes the route; a removed default leaves none.
fn remove(
    read: &impl Fn(&str) -> Option<String>,
    id: &str,
    out: &mut impl Write,
) -> Result<(), RouteCliError> {
    let id = id_arg(id)?;
    let (ledger, tenant) = open(read)?;
    ledger.remove_route(&tenant, id).map_err(from_exec)?;
    say(out, &json!({ "route": id, "removed": true }))
}

/// `larry route default <id>`: makes the route the tenant's one default.
fn make_default(
    read: &impl Fn(&str) -> Option<String>,
    id: &str,
    out: &mut impl Write,
) -> Result<(), RouteCliError> {
    let id = id_arg(id)?;
    let (ledger, tenant) = open(read)?;
    ledger.set_default_route(&tenant, id).map_err(from_exec)?;
    say(out, &json!({ "route": id, "default": true }))
}

/// `larry route resolve [<id>]`: the route a task naming `id` (or none) would
/// use, as the sealed report would record it.
fn show(
    read: &impl Fn(&str) -> Option<String>,
    named: Option<&str>,
    out: &mut impl Write,
) -> Result<(), RouteCliError> {
    let named = named.map(id_arg).transpose()?;
    let state = state_dir(read).map_err(from_exec)?;
    let ledger = Ledger::open(&state).map_err(from_exec)?;
    let route = resolve_for(&ledger, &state, named, read).map_err(from_exec)?;
    say(out, &route.seal())
}

/// Runs `larry route <args>` against the state directory `read` names, writing
/// its answer to `out`.
///
/// # Errors
///
/// [`RouteCliError`]: a usage refusal, a field the route check refuses, an
/// unknown id, a full table, or a store fault.
pub fn run(
    read: impl Fn(&str) -> Option<String>,
    args: &[&str],
    out: &mut impl Write,
) -> Result<(), RouteCliError> {
    match *args {
        ["add", id, ref flags @ ..] => add(&read, id, flags, out),
        ["list"] => list(&read, false, out),
        ["list", "--json"] => list(&read, true, out),
        ["remove", id] => remove(&read, id, out),
        ["default", id] => make_default(&read, id, out),
        ["resolve"] => show(&read, None, out),
        ["resolve", id] => show(&read, Some(id), out),
        _ => Err(RouteCliError::Usage("not a route command")),
    }
}
