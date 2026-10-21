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
        output::to_human_colored(&report, cli.no_command, colors_enabled(&cli))
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
