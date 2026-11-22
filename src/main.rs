use std::env;
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;

use portpeek::cli::Cli;
use portpeek::{Error, Report, find_listeners, output};

fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("portpeek: {error}");
            ExitCode::from(2)
        }
    }
}

/// How long `--kill` waits for signalled processes to release the port.
const KILL_TIMEOUT: Duration = Duration::from_secs(3);

fn run(cli: Cli) -> Result<bool, Error> {
    let report = find_listeners(cli.port, cli.protocol())?;
    let text = if cli.json {
        output::to_json(&report, cli.no_command)?
    } else {
        output::to_human_wrapped(
            &report,
            cli.no_command,
            colors_enabled(&cli),
            terminal_width(),
        )
    };

    write_stdout(&text)?;
    if report.is_empty() {
        return Ok(false);
    }

    if cli.kill {
        kill(&cli, &report)?;
    }
    Ok(true)
}

/// Run `--kill`. Messages go to stderr so `--json` output stays parseable.
#[cfg(unix)]
fn kill(cli: &Cli, report: &Report) -> Result<(), Error> {
    use portpeek::kill::{kill_listeners, send_sigterm};

    let mut stderr = io::stderr().lock();
    let outcome = kill_listeners(
        report,
        cli.force,
        &mut io::stdin().lock(),
        &mut stderr,
        send_sigterm,
    )?;

    // Wait for each signalled process to stop listening. A zombie keeps its
    // PID but not its sockets, so the port is what matters.
    let deadline = Instant::now() + KILL_TIMEOUT;
    let mut pending = outcome.signalled;
    while !pending.is_empty() {
        let current = find_listeners(cli.port, cli.protocol())?;
        pending.retain(|process| {
            let listening = current
                .listeners
                .iter()
                .any(|listener| listener.pid == Some(process.pid));
            if !listening {
                let _ = writeln!(stderr, "PID {} ({}) stopped.", process.pid, process.name);
            }
            listening
        });
        if pending.is_empty() || Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    for process in &pending {
        let _ = writeln!(
            stderr,
            "PID {0} ({1}) is still listening after {2}s; it may be shutting down or \
             ignoring SIGTERM. To force it: kill -9 {0}",
            process.pid,
            process.name,
            KILL_TIMEOUT.as_secs()
        );
    }

    let failed = outcome.failed + pending.len();
    if failed == 0 {
        Ok(())
    } else {
        let noun = if failed == 1 { "process" } else { "processes" };
        Err(Error::Backend(format!("could not kill {failed} {noun}")))
    }
}

#[cfg(not(unix))]
fn kill(_cli: &Cli, _report: &Report) -> Result<(), Error> {
    Err(Error::UnsupportedPlatform)
}

fn colors_enabled(cli: &Cli) -> bool {
    !cli.no_color
        && io::stdout().is_terminal()
        && env::var_os("NO_COLOR").is_none()
        && env::var_os("TERM").is_none_or(|term| term != "dumb")
}

/// Width available for tables: `COLUMNS` if set, else the terminal size.
///
/// Piped output without `COLUMNS` is not wrapped, so long commands stay on one
/// line for `grep` and friends.
fn terminal_width() -> Option<usize> {
    env::var("COLUMNS")
        .ok()
        .and_then(|columns| columns.trim().parse::<usize>().ok())
        .filter(|&columns| columns > 0)
        .or_else(|| {
            if io::stdout().is_terminal() {
                tty_columns()
            } else {
                None
            }
        })
}

#[cfg(unix)]
fn tty_columns() -> Option<usize> {
    // SAFETY: `winsize` is plain old data, and TIOCGWINSZ only writes into the
    // struct we pass.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &raw mut size) };
    (result == 0 && size.ws_col > 0).then(|| usize::from(size.ws_col))
}

#[cfg(not(unix))]
fn tty_columns() -> Option<usize> {
    None
}

fn write_stdout(text: &str) -> Result<(), Error> {
    let mut stdout = io::stdout().lock();
    match stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => Ok(()),
        // A downstream `head` or `grep -q` closing the pipe is not an error.
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(Error::Io(error)),
    }
}
