//! `larry route` and the route a task resolves to (larry#201). A route is data a
//! tenant owns in the state directory's store: `larry route add` stores one,
//! `list` shows them, `remove` and `default` change the table, and `resolve`
//! shows which route a task naming an id (or none) would use. The key is never
//! an argument and never stored: a route names the `LARRY_KEY_<TAG>_*` variable
//! that holds it, under the tag its own tenant's digest names, and the variable
//! is read only when a task reaches the model (larry#328).

use std::fmt;
use std::io::{self, Write};
use std::path::Path;

use lgwks_macros::Error;
use lgwks_std::json::{Value, json};

use super::task::Task;
use super::{ExecError, Ledger, Settings, state_dir};
use crate::eval::PriceFault;
use crate::model::{
    Draft, Kind, Policy, Route, RouteError, Selection, Table, env_route, is_route_id, query_of,
    resolve, select,
};
use crate::rerank::Scorer;
#[cfg(feature = "rerank")]
use crate::rerank::{Candidate, Deadline, RerankError};

/// Why `larry route` did not finish.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RouteCliError {
    /// The arguments are not a `larry route` command. The synopsis lives in
    /// this one message (larry#543): the derive renders tuple placeholders
    /// onto its extra arguments, so a `{USAGE}` tail would print twice.
    #[error(
        "route: {0}; usage: larry route add <id> --url <url> --model <model> --key-env <LARRY_KEY_NAME> --residency <CC> [--kind <openai-chat|anthropic-messages|openai-responses>] [--temperature <t>] [--request-bytes <n>] [--prompt <template>] [--describe <text>] [--default] | price <id> --in <dollars> --out <dollars> | list [--json] | remove <id> | default <id> | resolve [<id>]"
    )]
    Usage(&'static str),
    /// An argument is not UTF-8 text.
    #[error("route: every argument must be UTF-8 text")]
    NotUtf8,
    /// `route add` needs a flag it was not given.
    #[error("route add needs {flag}")]
    Missing {
        /// The flag.
        flag: &'static str,
    },
    /// A numeric flag holds something that is not a number.
    #[error("route {flag} is not a number: {reason}")]
    BadNumber {
        /// The flag.
        flag: &'static str,
        /// Why the text is not a number, in the parser's words.
        reason: String,
    },
    /// A `route price` rate the price grammar refuses.
    #[error("route price {flag} must be dollars per million tokens like 0.30, at most 10000")]
    Price {
        /// The flag that named the rate.
        flag: &'static str,
        /// Why the grammar refused it.
        #[source]
        fault: PriceFault,
    },
    /// A route, a route id or a resolution was refused.
    #[error("{}{__field_0}", route_prefix(__field_0))]
    Route(#[source] RouteError),
    /// The state directory or the store failed.
    #[error("route: {0}")]
    Exec(#[source] ExecError),
    /// Standard output could not be written.
    #[error("route output unwritable: {0}")]
    Output(#[source] io::Error),
}

/// What a route refusal is prefixed with: nothing when the refusal already
/// opens with `route <field>`, else `route: `, so none reaches the user bare.
/// The classification lives on the refusal itself
/// ([`RouteError::opens_with_route`]), listed once with no wildcard.
const fn route_prefix(cause: &RouteError) -> &'static str {
    if cause.opens_with_route() {
        ""
    } else {
        "route: "
    }
}

/// A refusal of the route table as the command's own, any other fault as the store's.
fn from_exec(error: ExecError) -> RouteCliError {
    match error {
        ExecError::Route(cause) => RouteCliError::Route(cause),
        other => RouteCliError::Exec(other),
    }
}

/// The tenant the caller selected, as the text every route row is keyed by:
/// `LARRY_TENANT` when it names one, else the store's default (larry#211).
fn tenant_text(
    ledger: &Ledger,
    state: &Path,
    read: &impl Fn(&str) -> Option<String>,
) -> Result<String, ExecError> {
    ledger.selected_tenant(state, read).map(|(_, text)| text)
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
    let tenant = tenant_text(ledger, state, read)?;
    let table = ledger.routes(&tenant)?;
    let environment = if table.lookup(named).is_some() {
        None
    } else {
        env_route(read).map_err(ExecError::ModelConfig)?
    };
    resolve(&tenant, &table, named, environment).map_err(ExecError::Route)
}

/// Builds the shipped ranker once, or names why it cannot.
#[cfg(feature = "rerank")]
type MakeScorer = Box<dyn FnMut() -> Result<Box<dyn Scorer>, RouteError>>;

/// A scorer that builds the shipped local ranker on its first scored
/// candidate, never on construction: a tenant whose choice needs no ranking
/// never fails for a missing model or home directory, and a missing engine
/// fails only the ranking that needs it, exactly as if no engine had been
/// built (larry#202).
#[cfg(feature = "rerank")]
struct LazyScorer {
    /// Builds the engine; taken on first use.
    make: Option<MakeScorer>,
    /// The engine once built.
    engine: Option<Box<dyn Scorer>>,
}

#[cfg(feature = "rerank")]
impl LazyScorer {
    /// A scorer that builds the shipped local ranker on first use.
    fn local() -> Self {
        Self {
            make: Some(Box::new(|| {
                let scorer: Box<dyn Scorer> =
                    Box::new(crate::rerank::local_scorer().map_err(|cause| {
                        RouteError::NoRanker {
                            detail: format!(
                                "{cause}; fetch the pinned model and retry, or name the route"
                            ),
                        }
                    })?);
                Ok(scorer)
            })),
            engine: None,
        }
    }
}

#[cfg(feature = "rerank")]
impl Scorer for LazyScorer {
    fn score(
        &mut self,
        query: &str,
        rows: &[Candidate],
        batch: usize,
        deadline: Deadline,
    ) -> Result<Vec<f64>, RerankError> {
        match self.engine {
            Some(ref mut engine) => engine.score(query, rows, batch, deadline),
            None => {
                let mut make = self.make.take().ok_or_else(|| {
                    RerankError::Engine(String::from("the route ranker is unavailable"))
                })?;
                let engine = make().map_err(|cause| match cause {
                    RouteError::NoRanker { detail } => RerankError::Engine(detail),
                    _ => RerankError::Engine(cause.to_string()),
                })?;
                self.engine
                    .insert(engine)
                    .score(query, rows, batch, deadline)
            }
        }
    }
}

/// The route a briefed task sends its model calls to: the named route when the
/// task names one (resolved exactly as the configured resolver does), else the
/// best-ranked eligible route. An explicit name always wins; without one the
/// transport and key binding gates, then the residency rule and the budget
/// filter before the shipped local ranker scores the survivors by meaning,
/// and without the `rerank` feature the eligible default wins (larry#202).
/// Nothing is sent to choose it.
///
/// # Errors
///
/// [`ExecError::Route`] for an unknown name, a malformed ranking knob, no
/// eligible route or an unavailable ranker; [`ExecError::ModelConfig`] for an
/// environment route that cannot be read.
pub(super) fn select_for(
    ledger: &Ledger,
    state: &Path,
    named: Option<&str>,
    query: Option<&str>,
    read: &impl Fn(&str) -> Option<String>,
) -> Result<Selection, ExecError> {
    let tenant = tenant_text(ledger, state, read)?;
    let table = ledger.routes(&tenant)?;
    let environment = if table.lookup(named).is_some() {
        None
    } else {
        env_route(read).map_err(ExecError::ModelConfig)?
    };
    let policy = Policy::parse(read).map_err(ExecError::Route)?;
    // The engine builds on its first scored candidate, never on construction:
    // a tenant whose choice needs no ranking never fails for a missing model
    // or home directory, and a missing engine fails only the ranking that
    // needs it, exactly as if no engine had been built (larry#202).
    #[cfg(feature = "rerank")]
    let mut engine: Option<Box<dyn Scorer>> = if named.is_none() {
        let scorer: Box<dyn Scorer> = Box::new(LazyScorer::local());
        Some(scorer)
    } else {
        None
    };
    #[cfg(not(feature = "rerank"))]
    let mut engine: Option<Box<dyn Scorer>> = None;
    // The scorer borrows the engine only for the call below, so each arm
    // ends the borrow before the choice returns. The query stays an option
    // into `select`: a named task resolves before the ranker runs, and an
    // unnamed one ranks only on the text `task_select` built from its brief.
    let choice = match engine.as_mut() {
        Some(held) => {
            let scorer: &mut dyn Scorer = &mut **held;
            select(
                &tenant,
                &table,
                named,
                environment,
                query,
                &policy,
                Some(scorer),
            )
        }
        None => select(&tenant, &table, named, environment, query, &policy, None),
    };
    choice.map_err(ExecError::Route)
}

/// The route a briefed task sends its model calls to: the named route when
/// the task names one, else the best-ranked eligible route, with the
/// residency and budget filters before the ranker and the choice sealed
/// beside the route for the replay to answer from (larry#202).
///
/// # Errors
///
/// [`ExecError::Route`] for an unknown name, a malformed ranking knob, no
/// eligible route or an unavailable ranker; [`ExecError::ModelConfig`] for an
/// environment route that cannot be read.
pub(crate) fn task_select(
    settings: &Settings,
    ledger: &Ledger,
    task: &Task,
) -> Result<Selection, ExecError> {
    let query = match (task.route(), task.brief()) {
        (None, Some(brief)) => Some(query_of(brief.goal(), brief.files(), task.acceptance())),
        _ => None,
    };
    select_for(
        ledger,
        settings.state_dir(),
        task.route(),
        query.as_deref(),
        &|name: &str| (settings.read)(name),
    )
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
    /// The provider kind: `openai-chat` unless `--kind` names another.
    kind: Option<&'arg str>,
    /// `--prompt`.
    prompt: Option<&'arg str>,
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
            "--kind" => &mut parsed.kind,
            "--prompt" => &mut parsed.prompt,
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
    let tenant = tenant_text(&ledger, &state, read).map_err(from_exec)?;
    Ok((ledger, tenant))
}

/// `larry route add <id> ...`: checks every field, refuses a key variable that
/// is not this tenant's own, then stores the route and prints the exact variable
/// name the child must export for it.
fn add(
    read: &impl Fn(&str) -> Option<String>,
    id: &str,
    flags: &[&str],
    out: &mut impl Write,
) -> Result<(), RouteCliError> {
    let parsed = parse_flags(flags)?;
    // A route added without `--describe` has the empty description, and one
    // added without `--kind` is an OpenAI chat route.
    let mut describe = "";
    if let Some(text) = parsed.describe {
        describe = text;
    }
    let kind = match parsed.kind {
        Some(name) => Kind::parse(name).ok_or(RouteCliError::Route(RouteError::BadKind))?,
        None => Kind::OpenAiChat,
    };
    let route = Route::new(&Draft {
        id,
        kind,
        base_url: required(parsed.url, "--url")?,
        model: required(parsed.model, "--model")?,
        key_env: required(parsed.key_env, "--key-env")?,
        temperature: number(parsed.temperature, "--temperature")?,
        request_bytes: number(parsed.request_bytes, "--request-bytes")?,
        residency: Some(required(parsed.residency, "--residency")?),
        prompt: parsed.prompt,
        describe,
    })
    .map_err(RouteCliError::Route)?;
    // The binding is checked before the store opens, so a refused add writes
    // nothing (larry#328).
    let state = state_dir(read).map_err(from_exec)?;
    let ledger = Ledger::open(&state).map_err(from_exec)?;
    let tenant = tenant_text(&ledger, &state, read).map_err(from_exec)?;
    if let Some(refusal) = route.key_env_refusal(&tenant) {
        return Err(RouteCliError::Route(refusal));
    }
    ledger
        .put_route(&tenant, &route, parsed.make_default)
        .map_err(from_exec)?;
    say(
        out,
        &json!({
            "route": route.id(),
            "saved": true,
            "made_default": parsed.make_default,
            "key_env": route.key_env(),
        }),
    )
}

/// `larry route price <id> --in <dollars> --out <dollars>`: sets the route's
/// versioned price table in micro-dollars per million tokens, bumping its
/// revision, and prints it. Only this command writes prices; `route add`
/// never does.
fn price(
    read: &impl Fn(&str) -> Option<String>,
    id: &str,
    flags: &[&str],
    out: &mut impl Write,
) -> Result<(), RouteCliError> {
    let id = id_arg(id)?;
    // Exactly the two rates, each with its value, nothing else.
    let mut input: Option<&str> = None;
    let mut output: Option<&str> = None;
    let mut rest = flags.iter().copied();
    while let Some(flag) = rest.next() {
        let slot = match flag {
            "--in" => &mut input,
            "--out" => &mut output,
            _ => return Err(RouteCliError::Usage("unknown flag")),
        };
        let value = rest
            .next()
            .ok_or(RouteCliError::Usage("a flag needs its value"))?;
        if slot.replace(value).is_some() {
            return Err(RouteCliError::Usage("a flag was given twice"));
        }
    }
    let (Some(input), Some(output)) = (input, output) else {
        return Err(RouteCliError::Usage("route price needs --in and --out"));
    };
    let input = price_rate(input, "--in")?;
    let output = price_rate(output, "--out")?;
    let (ledger, tenant) = open(read)?;
    ledger
        .set_route_price(&tenant, id, input, output)
        .map_err(from_exec)?;
    let table = ledger.routes(&tenant).map_err(from_exec)?;
    let held = table
        .lookup(Some(id))
        .ok_or_else(|| RouteCliError::Route(RouteError::NoSuchId { id: id.to_owned() }))?;
    say(
        out,
        &json!({
            "route": id,
            "priced": true,
            "price_version": held.price_version(),
            "price_input_micros": held.price_input(),
            "price_output_micros": held.price_output(),
        }),
    )
}

/// `text` as micro-dollars per million tokens, or the price refusal naming `flag`.
fn price_rate(text: &str, flag: &'static str) -> Result<u64, RouteCliError> {
    crate::eval::parse_price(text.trim()).map_err(|fault| RouteCliError::Price { flag, fault })
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
        "prompt_hash": route.prompt_hash(),
        "price_version": route.price_version(),
        "price_input_micros": route.price_input(),
        "price_output_micros": route.price_output(),
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
                "{}\t{}\t{}\t{residency}{marker}\t{}",
                route.id(),
                route.model(),
                route.host(),
                route.kind().name(),
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
/// use from configuration alone, as the sealed report would record it. A
/// briefed task that names none ranks its eligible routes by meaning instead
/// (larry#202), which needs the task and is not shown here.
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
        ["price", id, ref flags @ ..] => price(&read, id, flags, out),
        ["list"] => list(&read, false, out),
        ["list", "--json"] => list(&read, true, out),
        ["remove", id] => remove(&read, id, out),
        ["default", id] => make_default(&read, id, out),
        ["resolve"] => show(&read, None, out),
        ["resolve", id] => show(&read, Some(id), out),
        _ => Err(RouteCliError::Usage("not a route command")),
    }
}

#[cfg(test)]
mod tests {
    use super::{PriceFault, RouteCliError};
    use crate::model::RouteError;

    /// The `larry route` text of one refusal.
    fn said(error: &RouteCliError) -> String {
        error.to_string()
    }

    /// A field refusal names `route` once; a row that names no field keeps the
    /// CLI's context prefix once (larry#228).
    #[test]
    fn every_route_refusal_names_route_once() {
        let field = said(&RouteCliError::Route(RouteError::BadKeyEnv));
        assert!(
            field.starts_with("route key-env must name a variable"),
            "{field}"
        );
        assert_eq!(field.matches("route").count(), 1, "{field}");
        let outside = said(&RouteCliError::Route(RouteError::KeyEnvOutsideNamespace));
        assert!(outside.starts_with("route key-env must start"), "{outside}");
        assert_eq!(outside.matches("route").count(), 1, "{outside}");
        // larry#328: another tenant's variable is a field refusal too.
        let bound = said(&RouteCliError::Route(RouteError::KeyEnvNotThisTenants {
            variable: String::from("LARRY_KEY_THEIRS"),
            prefix: String::from("LARRY_KEY_1A2B3C4D5E6F7A8B_"),
        }));
        assert!(
            bound.starts_with("route key-env LARRY_KEY_THEIRS is not this tenant's"),
            "{bound}"
        );
        assert_eq!(bound.matches("route").count(), 1, "{bound}");
        let numbered = said(&RouteCliError::Route(RouteError::BadTemperature));
        assert!(numbered.starts_with("route temperature must"), "{numbered}");
        assert_eq!(numbered.matches("route").count(), 1, "{numbered}");
        let kind = said(&RouteCliError::Route(RouteError::BadKind));
        assert!(kind.starts_with("route kind must be one of"), "{kind}");
        assert_eq!(kind.matches("route").count(), 1, "{kind}");
        let prompt = said(&RouteCliError::Route(RouteError::BadPrompt));
        assert!(prompt.starts_with("route prompt must be"), "{prompt}");
        assert_eq!(prompt.matches("route").count(), 1, "{prompt}");
        let priced = said(&RouteCliError::Route(RouteError::BadPrice {
            field: "input",
        }));
        assert!(priced.starts_with("route input price must be"), "{priced}");
        assert_eq!(priced.matches("route").count(), 1, "{priced}");
        let rated = said(&RouteCliError::Price {
            flag: "--in",
            fault: PriceFault::Malformed,
        });
        assert!(rated.starts_with("route price --in must be"), "{rated}");
        assert_eq!(rated.matches("route").count(), 1, "{rated}");
        // larry#202: the selection knobs refuse as fields too.
        let rule = said(&RouteCliError::Route(RouteError::BadResidencyRule));
        assert!(
            rule.starts_with("route residency rule (LARRY_RESIDENCY) must be"),
            "{rule}"
        );
        assert_eq!(rule.matches("route").count(), 1, "{rule}");
        let seconds = said(&RouteCliError::Route(RouteError::BadRankSeconds));
        assert!(
            seconds.starts_with("route rank seconds (LARRY_ROUTE_RANK_SECONDS) must be"),
            "{seconds}"
        );
        assert_eq!(seconds.matches("route").count(), 1, "{seconds}");
        // larry#295: a refusal that names no field, such as `route remove r9` on an
        // unknown id, carries the `route` word by default.
        for row in [
            RouteError::NoSuchId {
                id: String::from("r9"),
            },
            RouteError::NoRoute,
            RouteError::NoEligibleRoute,
            RouteError::NoRanker {
                detail: String::from("the pinned model is missing"),
            },
            RouteError::TableFull { limit: 256 },
            RouteError::Damaged {
                id: String::from("r1"),
                cause: None,
            },
        ] {
            let text = said(&RouteCliError::Route(row));
            assert!(text.starts_with("route: "), "{text}");
            assert!(!text.starts_with("route: route"), "{text}");
        }
    }

    /// A successful add prints the route id, that it was saved, whether it is the
    /// default, and the exact key variable to export (larry#328).
    #[test]
    fn a_saved_route_prints_its_key_variable() -> Result<(), Box<dyn std::error::Error>> {
        let state = crate::executor::sim_tree::scratch("larry-route-add")?;
        let dir = state.to_string_lossy().into_owned();
        let ledger = super::Ledger::open(&state)?;
        let read = move |name: &str| (name == "LARRY_STATE_DIR").then(|| dir.clone());
        let tenant = super::tenant_text(&ledger, &state, &read)
            .map_err(|error| format!("tenant: {error}"))?;
        let key_env = format!("{}MAIN", crate::model::tenant_namespace(&tenant));
        let mut args = vec!["add", "main", "--url", "https://gateway.test:1/v1"];
        args.extend(["--model", "m", "--key-env", key_env.as_str()]);
        args.extend(["--residency", "CA"]);
        let mut out = Vec::new();
        super::run(read, &args, &mut out)?;
        let text = String::from_utf8(out)?;
        assert!(text.contains("\"saved\":true"), "{text}");
        assert!(text.contains(&key_env), "the exact variable: {text}");
        Ok(())
    }
}
