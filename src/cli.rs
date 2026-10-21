use clap::Parser;

use crate::model::Protocol;

/// Find the process listening on a local port, including its command line.
#[derive(Debug, Parser)]
#[command(
    name = "portpeek",
    version,
    about = "Find the process listening on a local port",
    long_about = None
)]
pub struct Cli {
    /// Port to inspect (1-65535)
    #[arg(value_name = "PORT", value_parser = clap::value_parser!(u16).range(1..))]
    pub port: u16,

    /// Inspect UDP sockets instead of TCP
    #[arg(long)]
    pub udp: bool,

    /// Print JSON instead of a human-readable report
    #[arg(long)]
    pub json: bool,

    /// Omit process command lines (useful when sharing output)
    #[arg(long)]
    pub no_command: bool,

    /// Disable ANSI colors in human-readable output
    #[arg(long)]
    pub no_color: bool,
}

impl Cli {
    #[must_use]
    pub const fn protocol(&self) -> Protocol {
        if self.udp {
            Protocol::Udp
        } else {
            Protocol::Tcp
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_port_and_flags() {
        let cli = Cli::try_parse_from(["portpeek", "8080", "--udp", "--json"]).unwrap();
        assert_eq!(cli.port, 8080);
        assert_eq!(cli.protocol(), Protocol::Udp);
        assert!(cli.json);
        assert!(!cli.no_command);
    }

    #[test]
    fn defaults_to_tcp() {
        let cli = Cli::try_parse_from(["portpeek", "3000"]).unwrap();
        assert_eq!(cli.protocol(), Protocol::Tcp);
        assert!(!cli.no_color);
    }

    #[test]
    fn parses_no_color() {
        let cli = Cli::try_parse_from(["portpeek", "3000", "--no-color"]).unwrap();
        assert!(cli.no_color);
    }

    #[test]
    fn rejects_port_zero() {
        assert!(Cli::try_parse_from(["portpeek", "0"]).is_err());
    }

    #[test]
    fn rejects_out_of_range_port() {
        assert!(Cli::try_parse_from(["portpeek", "65536"]).is_err());
        assert!(Cli::try_parse_from(["portpeek", "not-a-port"]).is_err());
    }
}
