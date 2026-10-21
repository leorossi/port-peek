use std::fmt::Write as _;

use crate::{Result, model::Report};

/// Render a human-readable report, grouped by process.
#[must_use]
pub fn to_human(report: &Report, no_command: bool) -> String {
    let normalized = report.clone().normalized();
    let report = &normalized;
    let mut out = String::new();
    let protocol = report.protocol.as_str().to_ascii_uppercase();

    if report.is_empty() {
        let _ = writeln!(out, "{protocol} {} · no listener", report.port);
        let _ = writeln!(
            out,
            "No process is listening on {protocol} port {}.",
            report.port
        );
        return out;
    }

    let count = report.process_count();
    let noun = if count == 1 { "process" } else { "processes" };
    let _ = writeln!(out, "{protocol} {} · {count} {noun}", report.port);

    for group in report.listeners.chunk_by(|a, b| a.pid == b.pid) {
        out.push('\n');
        let first = &group[0];

        match first.pid {
            Some(pid) => {
                let name = first.process.as_deref().unwrap_or("unknown");
                let _ = writeln!(out, "PID {pid} · {name}");
            }
            None => {
                let _ = writeln!(out, "PID ? · unknown process");
            }
        }

        if first.pid.is_some() {
            field(
                &mut out,
                "exe",
                &first.exe.as_ref().map_or_else(
                    || "(unavailable)".to_owned(),
                    |path| path.display().to_string(),
                ),
            );
        }

        let endpoints = group
            .iter()
            .map(crate::model::Listener::endpoint)
            .collect::<Vec<_>>()
            .join(", ");
        field(&mut out, "listening", &endpoints);

        if first.pid.is_some() {
            if !no_command {
                field(
                    &mut out,
                    "cmd",
                    first.command.as_deref().unwrap_or("(unavailable)"),
                );
            }
            field(
                &mut out,
                "cwd",
                &first.cwd.as_ref().map_or_else(
                    || "(unavailable)".to_owned(),
                    |path| path.display().to_string(),
                ),
            );
        }

        if first.pid.is_none() {
            field(
                &mut out,
                "note",
                "owner not visible; run with sudo to resolve",
            );
        }
    }

    out
}

fn field(out: &mut String, label: &str, value: &str) {
    let _ = writeln!(out, "  {label:<10} {value}");
}

/// Render the report as pretty JSON.
///
/// With `no_command`, the `argv` and `command` fields are omitted from every
/// listener so the output is safe to share.
pub fn to_json(report: &Report, no_command: bool) -> Result<String> {
    let mut report = report.clone().normalized();
    if no_command {
        for listener in &mut report.listeners {
            listener.argv = None;
            listener.command = None;
        }
    }

    let json = serde_json::to_string_pretty(&report)?;
    Ok(format!("{json}\n"))
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::*;
    use crate::model::{Listener, Protocol};

    fn sample() -> Report {
        Report::new(
            8080,
            Protocol::Tcp,
            vec![
                Listener {
                    pid: Some(42),
                    process: Some("node".into()),
                    exe: Some("/usr/local/bin/node".into()),
                    argv: Some(vec!["node".into(), "server.js".into()]),
                    command: Some("node server.js".into()),
                    cwd: Some("/tmp/app".into()),
                    protocol: Protocol::Tcp,
                    ip: "127.0.0.1".parse::<IpAddr>().unwrap(),
                    port: 8080,
                },
                Listener {
                    pid: Some(42),
                    process: Some("node".into()),
                    exe: Some("/usr/local/bin/node".into()),
                    argv: Some(vec!["node".into(), "server.js".into()]),
                    command: Some("node server.js".into()),
                    cwd: Some("/tmp/app".into()),
                    protocol: Protocol::Tcp,
                    ip: "::1".parse::<IpAddr>().unwrap(),
                    port: 8080,
                },
            ],
        )
    }

    #[test]
    fn groups_addresses_by_process() {
        let text = to_human(&sample(), false);
        assert!(text.contains("PID 42 · node"));
        assert!(text.contains("127.0.0.1:8080, [::1]:8080"));
        assert!(text.contains("cmd        node server.js"));
        assert!(text.contains("cwd        /tmp/app"));
    }

    #[test]
    fn no_command_removes_command_from_human_output() {
        let text = to_human(&sample(), true);
        assert!(!text.contains("cmd "));
        assert!(text.contains("cwd"));
    }

    #[test]
    fn no_command_removes_command_from_json() {
        let json = to_json(&sample(), true).unwrap();
        assert!(!json.contains("\"argv\""));
        assert!(!json.contains("\"command\""));
        assert!(json.contains("\"pid\": 42"));
    }

    #[test]
    fn reports_an_empty_port_clearly() {
        let report = Report::new(1234, Protocol::Tcp, Vec::new());
        let text = to_human(&report, false);
        assert!(text.contains("no listener"));
        assert!(text.contains("No process is listening on TCP port 1234."));
    }

    #[test]
    fn shows_a_hint_when_the_owner_is_not_visible() {
        let report = Report::new(
            8080,
            Protocol::Tcp,
            vec![Listener {
                pid: None,
                process: None,
                exe: None,
                argv: None,
                command: None,
                cwd: None,
                protocol: Protocol::Tcp,
                ip: "127.0.0.1".parse::<IpAddr>().unwrap(),
                port: 8080,
            }],
        );
        let text = to_human(&report, false);
        assert!(text.contains("PID ? · unknown process"));
        assert!(text.contains("run with sudo"));
    }

    #[cfg(unix)]
    #[test]
    fn json_serializes_non_utf8_paths_lossily() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let mut report = sample();
        let path = std::path::PathBuf::from(OsString::from_vec(b"/tmp/\xff".to_vec()));
        report.listeners[0].exe = Some(path.clone());
        report.listeners[0].cwd = Some(path);

        let json = to_json(&report, false).unwrap();
        assert!(json.contains("/tmp/\u{fffd}"));
    }
}
