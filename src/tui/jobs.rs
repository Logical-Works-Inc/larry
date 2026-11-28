//! Background commands behind `/ps` and `/stop` (larry#262, larry#556,
//! ADR 0128): the frame tracks every started command with its last five
//! output lines; `/ps` lists them, `/stop` ends the running ones through the
//! session's cancel path, the same one Esc uses, so their process groups die.
//! Quitting with running commands asks first, ends them, and quits when the
//! session ends. The state lives in [`super::pending`]; this module owns the
//! words and effects around it.

use super::app::{App, Effect, Key, Phase};

impl App {
    /// The quit hint while a session ends: it names the running commands, if any.
    pub(crate) fn quit_hint(&self) -> String {
        let running = self.pending.running_jobs();
        if running > 0 {
            format!(
                "{running} background command{} running. Larry quits when the session ends.",
                if running == 1 { "" } else { "s" }
            )
        } else {
            String::from("Larry quits when the session ends.")
        }
    }

    /// Names that no background command runs, over the footer until the next key.
    pub(crate) fn no_running(&mut self) {
        self.hint = Some(String::from("No background command is running."));
    }

    /// `/stop`: ends background commands by process group, through the session's
    /// cancel path, the same one Esc uses (larry#242).
    pub(crate) fn stop(&mut self) -> Vec<Effect> {
        match self.phase {
            Phase::Working(_) | Phase::Stopping(_) => {
                if self.pending.running_jobs() == 0 {
                    self.no_running();
                    return Vec::new();
                }
                self.interrupt(Key::Esc)
            }
            Phase::Idle | Phase::Asking(..) => {
                self.no_running();
                Vec::new()
            }
        }
    }

    /// `/quit` and the popup's fallback word while background commands run: with
    /// running commands the frame asks first, ends them, and quits when the
    /// session ends; with none it leaves at once.
    pub(crate) fn quit_or_leave(&mut self) -> Vec<Effect> {
        if self.phase != Phase::Idle && self.pending.running_jobs() > 0 {
            let running = self.pending.running_jobs();
            self.quitting = true;
            self.hint = Some(format!(
                "{running} background command{} running. Larry quits when the session ends; /stop ends them now.",
                if running == 1 { "" } else { "s" }
            ));
            if matches!(self.phase, Phase::Working(_)) {
                self.phase = Phase::Stopping(matches!(self.phase, Phase::Working(Some(_))));
                return vec![Effect::Interrupt];
            }
            return Vec::new();
        }
        vec![Effect::Quit]
    }
}
