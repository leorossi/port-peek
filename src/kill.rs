//! Terminate the processes listening on a port.
//!
//! The flow is kept free of global state so it can be tested: confirmation is
//! read from any `BufRead`, messages go to any `Write`, and the signal itself
//! is sent through a caller-supplied function.

use std::io::{self, BufRead, Write};

use crate::model::{Listener, Report};

/// A process that `--kill` sent a termination signal to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signalled {
    pub pid: u32,
    pub name: String,
}

/// What happened to each process group in a report.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Processes that were sent `SIGTERM` (or had already exited).
    pub signalled: Vec<Signalled>,
    /// Processes the user chose not to kill.
    pub declined: usize,
    /// Processes that could not be killed, with the reason already reported.
    pub failed: usize,
}

/// Ask for confirmation (unless `force`) and terminate each listening process.
///
/// `terminate` sends the signal; it is [`send_sigterm`] in the CLI. Messages
/// and prompts are written to `out`, answers are read from `input`. A process
/// is killed only after an explicit `y`/`yes`; end of input means no.
pub fn kill_listeners<R, W, F>(
    report: &Report,
    force: bool,
    input: &mut R,
    out: &mut W,
    mut terminate: F,
) -> io::Result<Outcome>
where
    R: BufRead,
    W: Write,
    F: FnMut(u32) -> io::Result<()>,
{
    let mut outcome = Outcome::default();

    for group in report.listeners.chunk_by(|a, b| a.pid == b.pid) {
        let first = &group[0];
        let endpoints = group
            .iter()
            .map(Listener::endpoint)
            .collect::<Vec<_>>()
            .join(", ");

        let Some(pid) = first.pid else {
            writeln!(
                out,
                "Cannot kill the owner of {endpoints}: it is not visible; run with sudo."
            )?;
            outcome.failed += 1;
            continue;
        };
        let name = first
            .process
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());

        if let Some(container) = &first.container {
            writeln!(
                out,
                "Not killing PID {pid} ({name}): it forwards the port for Docker container \
                 {0}. Stop the container instead: docker stop {0}",
                container.name
            )?;
            outcome.failed += 1;
            continue;
        }

        if pid <= 1 {
            writeln!(out, "Refusing to kill PID {pid} ({name}).")?;
            outcome.failed += 1;
            continue;
        }

        if !force
            && !confirm(
                input,
                out,
                &format!("Kill PID {pid} ({name}) on {endpoints}?"),
            )?
        {
            writeln!(out, "Left PID {pid} ({name}) running.")?;
            outcome.declined += 1;
            continue;
        }

        match terminate(pid) {
            Ok(()) => {
                writeln!(out, "Sent SIGTERM to PID {pid} ({name}).")?;
                outcome.signalled.push(Signalled { pid, name });
            }
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {
                writeln!(out, "PID {pid} ({name}) has already exited.")?;
                outcome.signalled.push(Signalled { pid, name });
            }
            Err(error) if error.raw_os_error() == Some(libc::EPERM) => {
                writeln!(
                    out,
                    "Cannot kill PID {pid} ({name}): permission denied; run with sudo."
                )?;
                outcome.failed += 1;
            }
            Err(error) => {
                writeln!(out, "Cannot kill PID {pid} ({name}): {error}")?;
                outcome.failed += 1;
            }
        }
    }

    Ok(outcome)
}

fn confirm<R: BufRead, W: Write>(input: &mut R, out: &mut W, question: &str) -> io::Result<bool> {
    write!(out, "{question} [y/N] ")?;
    out.flush()?;

    let mut answer = String::new();
    if input.read_line(&mut answer)? == 0 {
        // No answer (for example stdin is closed): print the missing newline.
        writeln!(out)?;
        return Ok(false);
    }
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// Send `SIGTERM` to `pid`.
#[cfg(unix)]
pub fn send_sigterm(pid: u32) -> io::Result<()> {
    let pid = libc::pid_t::try_from(pid)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "PID out of range"))?;
    // SAFETY: kill(2) takes plain integers and has no memory-safety effects.
    if unsafe { libc::kill(pid, libc::SIGTERM) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::model::{ContainerInfo, Protocol};

    fn listener(pid: Option<u32>, ip: &str) -> Listener {
        Listener {
            pid,
            process: Some("node".into()),
            exe: None,
            argv: None,
            command: None,
            cwd: None,
            protocol: Protocol::Tcp,
            ip: ip.parse().unwrap(),
            port: 8080,
            container: None,
        }
    }

    fn report(listeners: Vec<Listener>) -> Report {
        Report::new(8080, Protocol::Tcp, listeners).normalized()
    }

    fn run(
        report: &Report,
        force: bool,
        answers: &str,
        result: impl Fn(u32) -> io::Result<()>,
    ) -> (Outcome, Vec<u32>, String) {
        let calls = RefCell::new(Vec::new());
        let mut out = Vec::new();
        let outcome = kill_listeners(report, force, &mut answers.as_bytes(), &mut out, |pid| {
            calls.borrow_mut().push(pid);
            result(pid)
        })
        .unwrap();
        (outcome, calls.into_inner(), String::from_utf8(out).unwrap())
    }

    #[test]
    fn asks_once_per_process_and_kills_on_yes() {
        let report = report(vec![
            listener(Some(42), "127.0.0.1"),
            listener(Some(42), "::1"),
        ]);
        let (outcome, calls, out) = run(&report, false, "y\n", |_| Ok(()));
        assert_eq!(calls, [42]);
        assert_eq!(outcome.signalled.len(), 1);
        assert!(out.contains("Kill PID 42 (node) on 127.0.0.1:8080, [::1]:8080? [y/N]"));
        assert!(out.contains("Sent SIGTERM to PID 42 (node)."));
    }

    #[test]
    fn accepts_yes_in_any_case() {
        let report = report(vec![listener(Some(42), "127.0.0.1")]);
        let (_, calls, _) = run(&report, false, "  YES \n", |_| Ok(()));
        assert_eq!(calls, [42]);
    }

    #[test]
    fn declines_by_default_and_on_end_of_input() {
        let report = report(vec![listener(Some(42), "127.0.0.1")]);
        for answer in ["\n", "n\n", "nope\n", ""] {
            let (outcome, calls, out) = run(&report, false, answer, |_| Ok(()));
            assert!(calls.is_empty(), "killed on answer {answer:?}");
            assert_eq!(outcome.declined, 1);
            assert!(out.contains("Left PID 42 (node) running."));
        }
    }

    #[test]
    fn force_skips_the_prompt() {
        let report = report(vec![
            listener(Some(10), "127.0.0.1"),
            listener(Some(20), "127.0.0.1"),
        ]);
        let (outcome, calls, out) = run(&report, true, "", |_| Ok(()));
        assert_eq!(calls, [10, 20]);
        assert_eq!(outcome.signalled.len(), 2);
        assert!(!out.contains("[y/N]"));
    }

    #[test]
    fn answers_apply_to_each_process_in_order() {
        let report = report(vec![
            listener(Some(10), "127.0.0.1"),
            listener(Some(20), "127.0.0.1"),
        ]);
        let (outcome, calls, _) = run(&report, false, "n\ny\n", |_| Ok(()));
        assert_eq!(calls, [20]);
        assert_eq!(outcome.declined, 1);
        assert_eq!(outcome.signalled[0].pid, 20);
    }

    #[test]
    fn cannot_kill_an_unresolved_owner() {
        let report = report(vec![listener(None, "0.0.0.0")]);
        let (outcome, calls, out) = run(&report, true, "", |_| Ok(()));
        assert!(calls.is_empty());
        assert_eq!(outcome.failed, 1);
        assert!(out.contains("run with sudo"));
    }

    #[test]
    fn refuses_to_kill_a_docker_forwarder() {
        let mut forwarder = listener(Some(500), "::");
        forwarder.container = Some(ContainerInfo {
            id: "abc".into(),
            name: "web".into(),
            image: "nginx".into(),
            internal_port: 80,
            protocol: Protocol::Tcp,
            command: None,
            compose_project: None,
        });
        let (outcome, calls, out) = run(&report(vec![forwarder]), true, "", |_| Ok(()));
        assert!(calls.is_empty());
        assert_eq!(outcome.failed, 1);
        assert!(out.contains("docker stop web"));
    }

    #[test]
    fn refuses_to_kill_init() {
        let (outcome, calls, _) = run(&report(vec![listener(Some(1), "::")]), true, "", |_| Ok(()));
        assert!(calls.is_empty());
        assert_eq!(outcome.failed, 1);
    }

    #[test]
    fn reports_permission_errors() {
        let report = report(vec![listener(Some(42), "127.0.0.1")]);
        let (outcome, _, out) = run(&report, true, "", |_| {
            Err(io::Error::from_raw_os_error(libc::EPERM))
        });
        assert_eq!(outcome.failed, 1);
        assert!(out.contains("permission denied; run with sudo"));
    }

    #[test]
    fn treats_an_already_exited_process_as_done() {
        let report = report(vec![listener(Some(42), "127.0.0.1")]);
        let (outcome, _, out) = run(&report, true, "", |_| {
            Err(io::Error::from_raw_os_error(libc::ESRCH))
        });
        assert_eq!(outcome.failed, 0);
        assert_eq!(outcome.signalled.len(), 1);
        assert!(out.contains("already exited"));
    }
}
