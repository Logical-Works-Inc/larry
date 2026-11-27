//! What a task names about how it runs (larry#439): the shell its commands run
//! under and the per-command timeout. Each is read here; the tenant's ceilings
//! bound them before anything is cloned.

use super::{MAX_TIMEOUT_MS, Task, TaskError, is_shell};

impl Task {
    /// The shell the document names, when it names one.
    #[must_use]
    pub(crate) fn shell_program(&self) -> Option<&str> {
        self.shell.as_deref()
    }

    /// The per-command timeout the document names, in milliseconds.
    #[must_use]
    pub(crate) const fn timeout_ms(&self) -> Option<u64> {
        self.timeout_ms
    }

    /// The argv that runs for acceptance command `argv`: `<shell> -c <argv...>` when the
    /// document names a shell, which POSIX reads as the command string and then its positional
    /// parameters, else `argv` itself.
    #[must_use]
    pub(crate) fn argv_for(&self, argv: &[String]) -> Vec<String> {
        match self.shell_program() {
            Some(shell) => [&[shell.to_owned(), String::from("-c")], argv].concat(),
            None => argv.to_vec(),
        }
    }
}

/// The shell `object` names, when it names one: a program name or path of 1 to 256 bytes
/// with no whitespace, control byte or leading `-`.
pub(super) fn shell_of(object: &lgwks_std::json::Map) -> Result<Option<String>, TaskError> {
    let Some(entry) = object.get("shell") else {
        return Ok(None);
    };
    let text = entry
        .as_str()
        .ok_or(TaskError::WrongType { key: "shell" })?;
    if !is_shell(text) {
        return Err(TaskError::BadShell);
    }
    Ok(Some(text.to_owned()))
}

/// The milliseconds `object` names under `key`, when it names them: a whole number in
/// 1..=86400000, the executor's own bound on any wait.
pub(super) fn millis_of(
    object: &lgwks_std::json::Map,
    key: &'static str,
) -> Result<Option<u64>, TaskError> {
    let Some(entry) = object.get(key) else {
        return Ok(None);
    };
    match entry.as_u64() {
        Some(millis) if (1..=MAX_TIMEOUT_MS).contains(&millis) => Ok(Some(millis)),
        Some(_) => Err(TaskError::BadMillis { key }),
        None => Err(TaskError::WrongType { key }),
    }
}

#[cfg(test)]
mod tests {
    use lgwks_std::seeded::Seeded;

    use super::super::{Task, TaskError};

    /// A minimal task document with the raw JSON members `members` appended.
    fn document(members: &str) -> Vec<u8> {
        format!(r#"{{"repo":"/tmp/r","base":"HEAD","acceptance":[["c","a"]]{members}}}"#)
            .into_bytes()
    }

    /// larry#439: for every seed a timeout inside 1..=86400000 reads back exactly,
    /// zero and one past the bound are refused naming the key, a non-number is the wrong type,
    /// and a named shell composes `<shell> -c <argv...>` while no shell leaves the argv alone.
    #[test]
    fn sim_limits_read_back_or_refuse_by_key_for_every_seed()
    -> Result<(), Box<dyn std::error::Error>> {
        for seed in 0..64_u64 {
            let mut rng = Seeded::from_seed(seed);
            let millis = rng.below(86_400_000)?.saturating_add(1);
            let key = "timeout_ms";
            let task = Task::parse(&document(&format!(r#","{key}":{millis}"#)))?;
            assert_eq!(
                task.timeout_ms(),
                Some(millis),
                "seed {seed}: {key} reads back"
            );
            for bad in [0, 86_400_001] {
                assert!(
                    matches!(
                        Task::parse(&document(&format!(r#","{key}":{bad}"#))),
                        Err(TaskError::BadMillis { key: named }) if named == key
                    ),
                    "seed {seed}: {key} {bad} is refused by name"
                );
            }
            assert!(matches!(
                Task::parse(&document(&format!(r#","{key}":"{millis}""#))),
                Err(TaskError::WrongType { key: named }) if named == key
            ));
            let plain = Task::parse(&document(""))?;
            let argv = vec![String::from("c"), String::from("a")];
            assert_eq!(
                plain.argv_for(&argv),
                argv,
                "no shell runs the argv as it is"
            );
            let shelled = Task::parse(&document(r#","shell":"/bin/sh""#))?;
            assert_eq!(
                shelled.argv_for(&argv),
                ["/bin/sh", "-c", "c", "a"],
                "seed {seed}: a shell runs `<shell> -c <command> <args>`"
            );
            for bad in ["", "-x", "b a", "a\tb"] {
                let member = lgwks_std::json::json!(bad).to_string();
                assert!(
                    matches!(
                        Task::parse(&document(&format!(r#","shell":{member}"#))),
                        Err(TaskError::BadShell)
                    ),
                    "seed {seed}: shell {bad:?} is refused"
                );
            }
        }
        Ok(())
    }
}
