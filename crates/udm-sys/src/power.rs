//! Post-completion power actions.

use std::process::Command;

use crate::error::SysError;

/// Environment variable: when set to `1`, power actions are only logged.
pub const POWER_DRY_RUN_ENV: &str = "UDM_POWER_DRY_RUN";

/// A power action run after a queue completes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerAction {
    /// Shut the computer down.
    Shutdown,
    /// Suspend to RAM.
    Sleep,
    /// Suspend to disk.
    Hibernate,
}

impl PowerAction {
    /// Stable lower-case name (`shutdown`, `sleep`, `hibernate`).
    pub fn name(self) -> &'static str {
        match self {
            PowerAction::Shutdown => "shutdown",
            PowerAction::Sleep => "sleep",
            PowerAction::Hibernate => "hibernate",
        }
    }
}

/// Program and arguments that perform `a` on this platform.
pub(crate) fn power_command(
    a: PowerAction,
) -> Result<(&'static str, &'static [&'static str]), SysError> {
    #[cfg(windows)]
    {
        Ok(match a {
            PowerAction::Shutdown => ("shutdown", &["/s", "/t", "30"]),
            PowerAction::Sleep => ("rundll32.exe", &["powrprof.dll,SetSuspendState", "0,1,0"]),
            PowerAction::Hibernate => ("shutdown", &["/h"]),
        })
    }
    #[cfg(target_os = "macos")]
    {
        match a {
            PowerAction::Shutdown => Ok((
                "osascript",
                &["-e", "tell application \"System Events\" to shut down"],
            )),
            PowerAction::Sleep => Ok(("pmset", &["sleepnow"])),
            PowerAction::Hibernate => Err(SysError::Unsupported { what: "hibernate" }),
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Ok(match a {
            PowerAction::Shutdown => ("systemctl", &["poweroff"]),
            PowerAction::Sleep => ("systemctl", &["suspend"]),
            PowerAction::Hibernate => ("systemctl", &["hibernate"]),
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = a;
        Err(SysError::Unsupported { what: "power" })
    }
}

/// Performs a power action. With `UDM_POWER_DRY_RUN=1` it is only logged and `Ok` is returned.
pub fn perform_power_action(a: PowerAction) -> Result<(), SysError> {
    let dry_run = std::env::var(POWER_DRY_RUN_ENV).is_ok_and(|v| v == "1");
    perform_with(a, dry_run)
}

pub(crate) fn perform_with(a: PowerAction, dry_run: bool) -> Result<(), SysError> {
    let (program, args) = power_command(a)?;
    if dry_run {
        tracing::info!(action = a.name(), program, ?args, "power action (dry run)");
        return Ok(());
    }
    tracing::info!(action = a.name(), program, ?args, "power action");
    let mut cmd = Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        /// `CREATE_NO_WINDOW`: no console flashes up from the GUI.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let status = cmd.status().map_err(|e| SysError::Power {
        action: a.name(),
        detail: e.to_string(),
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(SysError::Power {
            action: a.name(),
            detail: status.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_never_executes() {
        for a in [
            PowerAction::Shutdown,
            PowerAction::Sleep,
            PowerAction::Hibernate,
        ] {
            match perform_with(a, true) {
                Ok(()) | Err(SysError::Unsupported { .. }) => {}
                Err(e) => panic!("{e:?}"),
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_commands() {
        assert_eq!(
            power_command(PowerAction::Shutdown).unwrap(),
            ("shutdown", &["/s", "/t", "30"][..])
        );
        assert_eq!(
            power_command(PowerAction::Sleep).unwrap(),
            (
                "rundll32.exe",
                &["powrprof.dll,SetSuspendState", "0,1,0"][..]
            )
        );
        assert_eq!(
            power_command(PowerAction::Hibernate).unwrap(),
            ("shutdown", &["/h"][..])
        );
    }

    #[test]
    fn names() {
        assert_eq!(PowerAction::Sleep.name(), "sleep");
        let m = SysError::Power {
            action: "sleep",
            detail: "x".into(),
        }
        .to_message();
        assert_eq!(m.key, "error-sys-power");
    }
}
