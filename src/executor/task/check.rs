//! The bounds a task document's repository, base and acceptance commands must
//! meet, split from the parser so each rule reads on its own (larry#85).

use super::{MAX_ARG_BYTES, MAX_ARGV, MAX_BASE_BYTES, MAX_COMMANDS, MAX_REPO_BYTES, TaskError};

/// The acceptance commands as owned argv arrays; any entry that is not an array of
/// strings is refused.
pub(super) fn owned_argvs(
    commands: &[lgwks_std::json::Value],
) -> Result<Vec<Vec<String>>, TaskError> {
    let mut acceptance: Vec<Vec<String>> = Vec::with_capacity(commands.len());
    for command in commands {
        let argv = command.as_array().ok_or(TaskError::AcceptanceNotArgv)?;
        let mut owned_argv: Vec<String> = Vec::with_capacity(argv.len());
        for item in argv {
            let text_item = item.as_str().ok_or(TaskError::AcceptanceNotArgv)?;
            owned_argv.push(text_item.to_owned());
        }
        acceptance.push(owned_argv);
    }
    Ok(acceptance)
}

/// Refuses a repository path or base the executor would hand to git unsafely.
pub(super) fn check_repo_and_base(repo_text: &str, base_text: &str) -> Result<(), TaskError> {
    if repo_text.is_empty()
        || repo_text.len() > MAX_REPO_BYTES
        || repo_text.contains('\0')
        || !std::path::Path::new(repo_text).is_absolute()
    {
        return Err(TaskError::BadRepo);
    }
    if base_text.is_empty()
        || base_text.len() > MAX_BASE_BYTES
        || base_text.starts_with('-')
        || base_text
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(TaskError::BadBase);
    }
    Ok(())
}

/// Refuses acceptance commands outside the command, argv and argument bounds. A task names 1
/// to [`MAX_COMMANDS`] commands: one that names none checks nothing, and its report would be
/// sealed accepted over whatever the model wrote (larry#395).
pub(super) fn check_acceptance(acceptance: &[Vec<String>]) -> Result<(), TaskError> {
    if acceptance.is_empty() || acceptance.len() > MAX_COMMANDS {
        return Err(TaskError::CommandCount {
            count: acceptance.len(),
        });
    }
    for (index, argv) in acceptance.iter().enumerate() {
        if argv.is_empty() {
            return Err(TaskError::EmptyArgv { index });
        }
        if argv.len() > MAX_ARGV {
            return Err(TaskError::ArgvTooLong {
                index,
                len: argv.len(),
            });
        }
        for item in argv {
            if item.len() > MAX_ARG_BYTES {
                return Err(TaskError::ArgTooLarge {
                    index,
                    len: item.len(),
                });
            }
            if item.contains('\0') {
                return Err(TaskError::ArgHasNul { index });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MAX_REPO_BYTES, check_repo_and_base};
    use crate::executor::TaskError;

    #[test]
    fn every_repository_boundary_is_refused_before_route_validation() {
        let too_long = format!("/{}", "r".repeat(MAX_REPO_BYTES));
        for repo in [
            String::new(),
            too_long,
            String::from("/tmp/a\0b"),
            String::from("relative"),
        ] {
            assert!(
                matches!(check_repo_and_base(&repo, "HEAD"), Err(TaskError::BadRepo)),
                "repository boundary {repo:?}"
            );
        }
    }
}
