use std::env;
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use clap::Parser;

use portpeek::cli::Cli;
use portpeek::{Error, find_listeners, output};

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
    Ok(!report.is_empty())
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
