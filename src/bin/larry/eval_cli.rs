//! `larry eval`, one priced arm over the matched task set (larry#87): its settings, its card
//! on standard output and the typed reason it stopped. Split from `src/bin/larry.rs` by
//! responsibility when a stopped arm's partial card took the path (larry#399); the table row in
//! that file calls [`eval_once`].

use larry_demo::eval::{EvalError, MAX_SET_BYTES, PriceError, Prices};
use larry_demo::executor::{ExecError, Settings, process_env};
use lgwks_macros::Error;

use super::{print_line, read_stdin};

/// Set bytes `larry eval` reads: one past the set bound, so an oversize set
/// reaches the parser's typed `TooLarge` refusal.
const SET_LIMIT: u64 = 4_194_305;

/// `SET_LIMIT` is the set bound plus one, checked at compile time.
const _: () = assert!(
    SET_LIMIT == 4_194_305 && MAX_SET_BYTES == 4_194_304,
    "SET_LIMIT is MAX_SET_BYTES plus one"
);

/// Why `larry eval` printed no complete scorecard; each variant keeps its cause.
#[derive(Debug, Error)]
pub(super) enum EvalRun {
    /// The executor settings, standard input or standard output failed.
    #[error("eval: {0}")]
    Exec(#[source] ExecError),
    /// A price setting is missing or refused.
    #[error("eval price: {0}")]
    Price(#[source] PriceError),
    /// The set was refused, or a task failed outside the arm's doing.
    #[error("{0}")]
    Eval(#[source] EvalError),
    /// The arm stopped partway and standard output refused its partial card too; both
    /// causes are kept, neither flattened.
    #[error("{stopped}; the card was not printed: {output}")]
    Unprinted {
        /// Why the arm stopped.
        #[source]
        stopped: Box<EvalError>,
        /// Why the card could not be printed.
        output: Box<ExecError>,
    },
}

/// `larry eval`: run every task of the matched set on standard input as one arm (the
/// model `LARRY_MODEL_*` pins, priced by `LARRY_EVAL_PRICE_IN`/`_OUT`) and print its
/// scorecard. Prices and the set are checked before the first model call (larry#87). A
/// fault partway prints the card of the outcomes before it, marked partial, and exits 1
/// with the cause (larry#399).
pub(super) fn eval_once() -> Result<(), EvalRun> {
    let settings =
        Settings::from_env(process_env().map_err(EvalRun::Exec)?).map_err(EvalRun::Exec)?;
    let prices = Prices::from_env(process_env().map_err(EvalRun::Exec)?).map_err(EvalRun::Price)?;
    let bytes = read_stdin(SET_LIMIT).map_err(EvalRun::Exec)?;
    let stopped = match larry_demo::eval::run(&bytes, &settings, prices) {
        Ok(card) => return print_line(card.json()).map_err(EvalRun::Exec),
        Err(stopped) => stopped,
    };
    if let Some(card) = stopped.card()
        && let Err(output) = print_line(card.json())
    {
        return Err(EvalRun::Unprinted {
            stopped: Box::new(stopped),
            output: Box::new(output),
        });
    }
    Err(EvalRun::Eval(stopped))
}
