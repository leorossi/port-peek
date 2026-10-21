use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use serde::Serialize;

/// Transport protocol of a listening socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Udp,
}

impl Protocol {
    /// Lowercase name used in output and JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One listening socket, optionally attributed to a process.
///
/// Fields are optional because a socket can be visible while its owning process
/// is not (for example when running without root privileges).
#[derive(Debug, Clone, Serialize)]
pub struct Listener {
    pub pid: Option<u32>,
    pub process: Option<String>,
    #[serde(serialize_with = "serialize_optional_path")]
    pub exe: Option<PathBuf>,
    /// Full argument vector as reported by the kernel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argv: Option<Vec<String>>,
    /// Shell-escaped rendering of `argv`, for convenience only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(serialize_with = "serialize_optional_path")]
    pub cwd: Option<PathBuf>,
    pub protocol: Protocol,
    pub ip: IpAddr,
    pub port: u16,
}

impl Listener {
    /// `ip:port`, with IPv6 addresses bracketed.
    #[must_use]
    pub fn endpoint(&self) -> String {
        SocketAddr::new(self.ip, self.port).to_string()
    }
}

fn serialize_optional_path<S>(
    path: &Option<PathBuf>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match path {
        Some(path) => serializer.serialize_some(&path.to_string_lossy()),
        None => serializer.serialize_none(),
    }
}

/// Result of looking up one port.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub port: u16,
    pub protocol: Protocol,
    pub listeners: Vec<Listener>,
}

impl Report {
    #[must_use]
    pub fn new(port: u16, protocol: Protocol, listeners: Vec<Listener>) -> Self {
        Self {
            port,
            protocol,
            listeners,
        }
    }

    /// Stable ordering and removal of duplicate bindings.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.listeners.sort_by_key(|listener| {
            (
                listener.pid.is_none(),
                listener.pid.unwrap_or(0),
                listener.ip,
                listener.port,
            )
        });
        self.listeners.dedup_by(|a, b| {
            a.pid == b.pid && a.ip == b.ip && a.port == b.port && a.protocol == b.protocol
        });
        self
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.listeners.is_empty()
    }

    /// Number of distinct processes (unresolved sockets count as one group).
    #[must_use]
    pub fn process_count(&self) -> usize {
        let pids: std::collections::BTreeSet<Option<u32>> =
            self.listeners.iter().map(|listener| listener.pid).collect();
        pids.len()
    }
}

/// Render an argv vector as a POSIX-shell-safe command line.
///
/// This reconstructs a command that is safe to paste into a shell. The exact
/// quoting used when the process was started cannot be recovered.
#[must_use]
pub fn shell_join(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_owned();
    }

    // Characters that are safe unquoted in a POSIX shell. Deliberately
    // conservative so the result is easy to read.
    if arg
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&byte))
    {
        return arg.to_owned();
    }

    format!("'{}'", arg.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listener(pid: Option<u32>, ip: &str) -> Listener {
        Listener {
            pid,
            process: Some("test".into()),
            exe: None,
            argv: None,
            command: None,
            cwd: None,
            protocol: Protocol::Tcp,
            ip: ip.parse().unwrap(),
            port: 8080,
        }
    }

    #[test]
    fn quotes_arguments_for_a_shell() {
        let argv = [
            "node".to_owned(),
            "server.js".to_owned(),
            "a b".to_owned(),
            "it's".to_owned(),
            String::new(),
        ];
        assert_eq!(shell_join(&argv), "node server.js 'a b' 'it'\\''s' ''");
    }

    #[test]
    fn normalizes_and_dedupes() {
        let report = Report::new(
            8080,
            Protocol::Tcp,
            vec![
                listener(None, "0.0.0.0"),
                listener(Some(20), "::1"),
                listener(Some(10), "127.0.0.1"),
                listener(Some(10), "127.0.0.1"),
            ],
        )
        .normalized();

        assert_eq!(report.listeners.len(), 3);
        assert_eq!(report.listeners[0].pid, Some(10));
        assert_eq!(report.listeners[1].pid, Some(20));
        assert_eq!(report.listeners[2].pid, None);
    }

    #[test]
    fn formats_ipv6_endpoints() {
        assert_eq!(listener(Some(1), "::1").endpoint(), "[::1]:8080");
    }

    #[test]
    fn counts_distinct_processes() {
        let report = Report::new(
            1,
            Protocol::Tcp,
            vec![listener(Some(1), "127.0.0.1"), listener(None, "::1")],
        );
        assert_eq!(report.process_count(), 2);
    }
}
