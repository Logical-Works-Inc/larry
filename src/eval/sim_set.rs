//! Seeded simulations of `larry eval`'s set checks (larry#87): every malformed set
//! gets its exact error before anything runs, and a valid set reads back exactly.
//! One seed drives every string, count and bound.

use std::error::Error;

use lgwks_std::json::{Map, Value, json};
use lgwks_std::seeded::Seeded;

use super::set::{SetError, TaskSet};
use crate::executor::TaskError;
use crate::json_at::At;
use crate::sim_error::SimError;

/// What every test returns.
type TestResult = Result<(), Box<dyn Error>>;

/// The letters a seeded word is drawn from.
const LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyz";

/// Origin file contents a seeded set draws from: empty, unicode, a script, right-to-left.
const CONTENTS: [&str; 4] = ["", "ünïcödé\n", "#!/bin/sh\necho 42\n", "\u{202e}rtl"];

/// A seeded lowercase word of 1 to 8 letters.
fn word(rng: &mut Seeded) -> Result<String, SimError> {
    let mut text = String::new();
    for _ in 0..=rng.index(8)? {
        let letter = LETTERS
            .get(rng.index(LETTERS.len())?)
            .ok_or(SimError::Missing("letters"))?;
        text.push(char::from(*letter));
    }
    Ok(text)
}

/// A seeded valid set of `count` tasks with unicode, right-to-left and empty origin files.
fn valid_set(rng: &mut Seeded, count: usize) -> Result<Value, SimError> {
    let mut tasks = Vec::with_capacity(count);
    for index in 0..count {
        let mut files = Map::new();
        for slot in 0..=rng.index(4)? {
            let content = CONTENTS
                .get(rng.index(CONTENTS.len())?)
                .ok_or(SimError::Missing("contents"))?;
            put(
                &mut files,
                &format!("{}/{slot}.txt", word(rng)?),
                json!(content),
            );
        }
        let write: Vec<String> = (0..=rng.index(3)?)
            .map(|slot| format!("out/{slot}.sh"))
            .collect();
        let acceptance: Vec<String> = (0..=rng.index(4)?)
            .map(|slot| format!("test -s out/{slot}.sh"))
            .collect();
        tasks.push(json!({
            "id": format!("{}-{index}", word(rng)?),
            "files": files,
            "goal": format!("goal {index}"),
            "write": write,
            "acceptance": acceptance,
        }));
    }
    Ok(json!({ "name": "matched-sim", "tasks": tasks }))
}

/// Sets `key` of `object` to `value`; a value it replaces is the valid one the
/// mistake overwrites, never an error.
fn put(object: &mut Map, key: &str, value: Value) {
    object.insert(String::from(key), value);
}

/// Task `index` of set `value`, to rewrite in place.
fn task_mut(value: &mut Value, index: usize) -> Option<&mut Map> {
    value.get_mut("tasks")?.get_mut(index)?.as_object_mut()
}

/// The set `value` after `edit` on its first task's object; `None` when it has none.
fn with_task(value: &Value, edit: fn(&mut Map)) -> Option<Vec<u8>> {
    let mut changed = value.clone();
    edit(task_mut(&mut changed, 0)?);
    Some(changed.to_string().into_bytes())
}

/// The error a set is refused with; `None` when it was accepted.
fn refusal(bytes: &[u8]) -> Option<SetError> {
    TaskSet::parse(bytes).err()
}

/// A whole set's bytes and the exact refusal they must draw.
type WholeMistake = (&'static [u8], fn(&SetError) -> bool);

/// One mistake made to task 0, and the exact refusal it must draw.
type Mistake = (fn(&mut Map), fn(&SetError) -> bool);

/// Every single mistake a fat-fingered author makes in one task.
const MISTAKES: [Mistake; 13] = [
    (
        |task| put(task, "cost", json!(0)),
        |error| matches!(*error, SetError::UnknownKey { index: Some(0), .. }),
    ),
    (
        |task| put(task, "id", json!("")),
        |error| matches!(*error, SetError::BadId { index: 0 }),
    ),
    (
        |task| put(task, "files", json!({})),
        |error| matches!(*error, SetError::FileCount { index: 0, count: 0 }),
    ),
    (
        |task| put(task, "files", json!({"../escape": "x"})),
        |error| matches!(*error, SetError::BadFile { index: 0, .. }),
    ),
    (
        |task| put(task, "files", json!({".git/config": "x"})),
        |error| matches!(*error, SetError::BadFile { index: 0, .. }),
    ),
    (
        |task| put(task, "files", json!({"big": "x".repeat(262_145)})),
        |error| matches!(*error, SetError::FileTooLarge { index: 0, .. }),
    ),
    (
        |task| put(task, "files", json!({"a": 7})),
        |error| {
            matches!(
                *error,
                SetError::WrongType {
                    index: Some(0),
                    key: "files"
                }
            )
        },
    ),
    (
        |task| put(task, "acceptance", json!([])),
        |error| matches!(*error, SetError::AcceptanceCount { index: 0, count: 0 }),
    ),
    (
        |task| put(task, "acceptance", json!(vec!["true"; 17])),
        |error| {
            matches!(
                *error,
                SetError::AcceptanceCount {
                    index: 0,
                    count: 17
                }
            )
        },
    ),
    (
        |task| put(task, "write", json!([])),
        |error| {
            matches!(
                *error,
                SetError::Task {
                    index: 0,
                    cause: TaskError::FileCount { count: 0 }
                }
            )
        },
    ),
    (
        |task| put(task, "write", json!(["a", "a"])),
        |error| {
            matches!(
                *error,
                SetError::Task {
                    index: 0,
                    cause: TaskError::BadFile { index: 1 }
                }
            )
        },
    ),
    (
        |task| put(task, "goal", json!(" ")),
        |error| {
            matches!(
                *error,
                SetError::Task {
                    index: 0,
                    cause: TaskError::BadGoal
                }
            )
        },
    ),
    (
        |task| put(task, "goal", json!(7)),
        |error| {
            matches!(
                *error,
                SetError::WrongType {
                    index: Some(0),
                    key: "goal"
                }
            )
        },
    ),
];

/// fat-finger, giant-and-empty and wrong-order: every single mistake in a seeded
/// valid set is refused with its exact error before anything runs. Catches a set
/// that runs with no acceptance (accepted for nothing), an origin file outside the
/// clone, a repeated id that would merge two tasks' scores, and an oversize set
/// read whole.
#[test]
fn sim_eval_fat_finger_sets_get_their_exact_error() -> TestResult {
    for seed in 0..64 {
        let mut rng = Seeded::from_seed(seed);
        let count = rng.index(4)?.saturating_add(2);
        let set = valid_set(&mut rng, count)?;
        assert!(
            refusal(set.to_string().as_bytes()).is_none(),
            "seed {seed}: the base set is valid"
        );
        for (index, &(edit, expected)) in MISTAKES.iter().enumerate() {
            let error = refusal(&with_task(&set, edit).ok_or("task 0")?).ok_or("accepted")?;
            assert!(
                expected(&error),
                "seed {seed}: mistake {index} gave {error}"
            );
        }
        let unknown_path =
            with_task(&set, |task| put(task, "path", json!("terminal"))).ok_or("task 0")?;
        assert!(
            matches!(
                refusal(&unknown_path),
                Some(SetError::UnknownPath { index: 0, .. })
            ),
            "seed {seed}: an unknown path is typed"
        );
        let mut twins = set.clone();
        let first = task_mut(&mut twins, 0)
            .and_then(|task| task.get("id"))
            .cloned();
        let twin = task_mut(&mut twins, 1).ok_or("task 1")?;
        put(twin, "id", first.ok_or("id 0")?);
        let error = refusal(twins.to_string().as_bytes()).ok_or("accepted twins")?;
        assert!(
            matches!(error, SetError::DuplicateId { index: 1 }),
            "seed {seed}: {error}"
        );
        let mut extra = set;
        put(
            extra.as_object_mut().ok_or("set object")?,
            "extra",
            json!(1),
        );
        let error = refusal(extra.to_string().as_bytes()).ok_or("accepted extra")?;
        assert!(
            matches!(error, SetError::UnknownKey { index: None, .. }),
            "{error}"
        );
    }
    let whole: [WholeMistake; 7] = [
        (br#"{"tasks":[]}"#, |error| {
            matches!(
                *error,
                SetError::WrongType {
                    index: None,
                    key: "name"
                }
            )
        }),
        (br#"{"name":"Bad Name","tasks":[]}"#, |error| {
            matches!(*error, SetError::BadName)
        }),
        (br#"{"name":"s","tasks":[]}"#, |error| {
            matches!(*error, SetError::TaskCount { count: 0 })
        }),
        (br#"{"name":"s","tasks":{}}"#, |error| {
            matches!(
                *error,
                SetError::WrongType {
                    index: None,
                    key: "tasks"
                }
            )
        }),
        (br#"{"name":"s","tasks":[7]}"#, |error| {
            matches!(*error, SetError::NotAnObject { index: Some(0) })
        }),
        (b"[1]", |error| {
            matches!(*error, SetError::NotAnObject { index: None })
        }),
        (b"{\"name\":", |error| {
            matches!(*error, SetError::NotJson(_))
        }),
    ];
    for (bytes, expected) in whole {
        let error = refusal(bytes).ok_or("accepted")?;
        assert!(expected(&error), "{error}");
    }
    let oversize = refusal(&vec![b' '; 4_194_305]).ok_or("accepted oversize")?;
    assert!(
        matches!(oversize, SetError::TooLarge { len: 4_194_305 }),
        "{oversize}"
    );
    Ok(())
}

/// monkey: seeded valid sets of 1 to 12 tasks with unicode, right-to-left and
/// empty origin files read back exactly, task by task, in order, and each task
/// builds the executor document naming its files. Catches a reordered or dropped
/// task, a mangled origin file, and a set check that disagrees with the executor.
#[test]
fn sim_eval_valid_sets_read_back_exactly() -> TestResult {
    for seed in 0..256 {
        let mut rng = Seeded::from_seed(seed);
        let count = rng.index(12)?.saturating_add(1);
        let value = valid_set(&mut rng, count)?;
        let set = TaskSet::parse(value.to_string().as_bytes())?;
        assert_eq!(set.name, "matched-sim", "seed {seed}: name");
        assert_eq!(set.tasks.len(), count, "seed {seed}: every task");
        for (index, task) in set.tasks.iter().enumerate() {
            let written = value.at(&format!("/tasks/{index}"))?;
            assert_eq!(
                &json!(task.id),
                written.at("/id")?,
                "seed {seed}: id {index}"
            );
            assert_eq!(
                &json!(task.goal),
                written.at("/goal")?,
                "seed {seed}: goal {index}"
            );
            assert_eq!(
                &json!(task.write),
                written.at("/write")?,
                "seed {seed}: write {index}"
            );
            assert_eq!(
                &json!(task.acceptance),
                written.at("/acceptance")?,
                "seed {seed}: acceptance {index}"
            );
            let files: Map = task
                .files
                .iter()
                .map(|pair| (pair.0.clone(), json!(pair.1)))
                .collect();
            assert_eq!(
                &Value::Object(files),
                written.at("/files")?,
                "seed {seed}: files {index}"
            );
            assert!(
                task.task("/origin").is_ok(),
                "seed {seed}: task {index} builds its document"
            );
        }
    }
    Ok(())
}
