use std::fmt::Write as _;

use crate::{Result, model::Report};

const RESET: &str = "\x1b[0m";
const STYLE_TITLE: &str = "1;36";
const STYLE_BORDER: &str = "2";
const STYLE_LABEL: &str = "1;36";
const STYLE_PID: &str = "1;33";
const STYLE_PROCESS: &str = "1";
const STYLE_ENDPOINT: &str = "36";
const STYLE_CONTAINER: &str = "1;35";
const STYLE_DIM: &str = "2";
const STYLE_NOTE: &str = "33";

/// Render a human-readable report, grouped by process.
#[must_use]
pub fn to_human(report: &Report, no_command: bool) -> String {
    to_human_colored(report, no_command, false)
}

/// Render a human-readable report, optionally with ANSI colors.
///
/// Colors are applied only when `color` is true. The caller should decide
/// whether stdout is a terminal and whether the user opted out.
#[must_use]
pub fn to_human_colored(report: &Report, no_command: bool, color: bool) -> String {
    let normalized = report.clone().normalized();
    let report = &normalized;
    let mut out = String::new();
    let protocol = report.protocol.as_str().to_ascii_uppercase();

    if report.is_empty() {
        let title = format!("{protocol} {} · no listener", report.port);
        let _ = writeln!(out, "{}", paint(&title, STYLE_TITLE, color));
        let message = format!(
            "No process is listening on {protocol} port {}.",
            report.port
        );
        let _ = writeln!(out, "{}", paint(&message, STYLE_DIM, color));
        return out;
    }

    let count = report.process_count();
    let noun = if count == 1 { "process" } else { "processes" };
    let title = format!("{protocol} {} · {count} {noun}", report.port);
    let _ = writeln!(out, "{}", paint(&title, STYLE_TITLE, color));
    out.push('\n');

    for (index, group) in report.listeners.chunk_by(|a, b| a.pid == b.pid).enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&process_table(group, no_command, color));
    }

    out
}

fn process_table(group: &[crate::model::Listener], no_command: bool, color: bool) -> String {
    let first = &group[0];
    let mut rows = Vec::new();

    rows.push(Row::new(
        "PID",
        first
            .pid
            .map_or_else(|| "?".to_owned(), |pid| pid.to_string()),
        STYLE_PID,
    ));
    rows.push(Row::new(
        "Process",
        first
            .process
            .clone()
            .unwrap_or_else(|| "unknown".to_owned()),
        STYLE_PROCESS,
    ));

    let endpoints = group
        .iter()
        .map(crate::model::Listener::endpoint)
        .collect::<Vec<_>>()
        .join(", ");
    rows.push(Row::new("Listening", endpoints, STYLE_ENDPOINT));

    if let Some(container) = &first.container {
        rows.push(Row::new(
            "Container",
            format!("{} ({})", container.name, container.short_id()),
            STYLE_CONTAINER,
        ));
        rows.push(Row::new("Image", container.image.clone(), ""));
        rows.push(Row::new(
            "Internal",
            format!("{}/{}", container.internal_port, container.protocol),
            "",
        ));
        if let Some(project) = &container.compose_project {
            rows.push(Row::new("Compose", project.clone(), ""));
        }
    }

    if first.pid.is_some() {
        rows.push(Row::new(
            "Executable",
            first.exe.as_ref().map_or_else(
                || "(unavailable)".to_owned(),
                |path| path.display().to_string(),
            ),
            "",
        ));

        if !no_command {
            let command = first
                .container
                .as_ref()
                .and_then(|container| container.command.as_deref())
                .or(first.command.as_deref())
                .unwrap_or("(unavailable)");
            rows.push(Row::new("Command", command.to_owned(), ""));
        }

        rows.push(Row::new(
            "Working dir",
            first.cwd.as_ref().map_or_else(
                || "(unavailable)".to_owned(),
                |path| path.display().to_string(),
            ),
            STYLE_DIM,
        ));
    } else {
        rows.push(Row::new(
            "Note",
            "owner not visible; run with sudo to resolve".to_owned(),
            STYLE_NOTE,
        ));
    }

    render_table(&rows, color)
}

struct Row {
    label: &'static str,
    value: String,
    style: &'static str,
}

impl Row {
    fn new(label: &'static str, value: String, style: &'static str) -> Self {
        Self {
            label,
            value,
            style,
        }
    }
}

fn render_table(rows: &[Row], color: bool) -> String {
    let label_width = rows
        .iter()
        .map(|row| row.label.chars().count())
        .max()
        .unwrap_or(0);
    let value_width = rows
        .iter()
        .map(|row| row.value.chars().count())
        .max()
        .unwrap_or(0);

    let label_border = "─".repeat(label_width + 2);
    let value_border = "─".repeat(value_width + 2);
    let mut out = String::new();

    let top = format!("┌{label_border}┬{value_border}┐");
    let _ = writeln!(out, "{}", paint(&top, STYLE_BORDER, color));

    for row in rows {
        let label = pad_to(row.label, label_width);
        let value = pad_to(&row.value, value_width);
        let _ = writeln!(
            out,
            "│ {} │ {} │",
            paint(&label, STYLE_LABEL, color),
            paint(&value, row.style, color)
        );
    }

    let bottom = format!("└{label_border}┴{value_border}┘");
    let _ = writeln!(out, "{}", paint(&bottom, STYLE_BORDER, color));
    out
}

fn pad_to(text: &str, width: usize) -> String {
    let length = text.chars().count();
    if length >= width {
        return text.to_owned();
    }

    let mut padded = String::with_capacity(text.len() + (width - length));
    padded.push_str(text);
    for _ in length..width {
        padded.push(' ');
    }
    padded
}

fn paint(text: &str, style: &str, enabled: bool) -> String {
    if enabled && !style.is_empty() {
        format!("\x1b[{style}m{text}{RESET}")
    } else {
        text.to_owned()
    }
}

/// Render the report as pretty JSON.
///
/// With `no_command`, the `argv` and `command` fields are omitted from every
/// listener, including the `command` inside Docker container metadata, so the
/// output is safe to share.
pub fn to_json(report: &Report, no_command: bool) -> Result<String> {
    let mut report = report.clone().normalized();
    if no_command {
        for listener in &mut report.listeners {
            listener.argv = None;
            listener.command = None;
            if let Some(container) = &mut listener.container {
                container.command = None;
            }
        }
    }

    let json = serde_json::to_string_pretty(&report)?;
    Ok(format!("{json}\n"))
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::*;
    use crate::model::{ContainerInfo, Listener, Protocol};

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
                    container: None,
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
                    container: None,
                },
            ],
        )
    }

    fn docker_sample() -> Report {
        Report::new(
            8000,
            Protocol::Tcp,
            vec![Listener {
                pid: Some(98560),
                process: Some("com.docker.backend".into()),
                exe: Some("/Applications/Docker.app/Contents/MacOS/com.docker.backend".into()),
                argv: Some(vec!["com.docker.backend".into(), "services".into()]),
                command: Some("com.docker.backend services".into()),
                cwd: Some("/Users/leo/Library/Containers/com.docker.docker/Data".into()),
                protocol: Protocol::Tcp,
                ip: "::".parse::<IpAddr>().unwrap(),
                port: 8000,
                container: Some(ContainerInfo {
                    id: "9f8cac51baba0000000000000000000000000000000000000000000000000000".into(),
                    name: "telemetry-demo-app".into(),
                    image: "telemetry-demo-app".into(),
                    internal_port: 8000,
                    protocol: Protocol::Tcp,
                    command: Some(
                        "entrypoint.sh php artisan serve --host=0.0.0.0 --port=8000".into(),
                    ),
                    compose_project: Some("telemetry-talk".into()),
                }),
            }],
        )
    }

    #[test]
    fn groups_addresses_by_process_in_a_table() {
        let text = to_human(&sample(), false);
        assert!(text.contains("PID"));
        assert!(text.contains("42"));
        assert!(text.contains("node"));
        assert!(text.contains("127.0.0.1:8080, [::1]:8080"));
        assert!(text.contains("Command"));
        assert!(text.contains("node server.js"));
        assert!(text.contains("Working dir"));
        assert!(text.contains("/tmp/app"));
        assert!(text.contains('┌'));
        assert!(text.contains('┘'));
    }

    #[test]
    fn renders_docker_container_metadata() {
        let text = to_human(&docker_sample(), false);
        assert!(text.contains("Container"));
        assert!(text.contains("telemetry-demo-app (9f8cac51baba)"));
        assert!(text.contains("Image"));
        assert!(text.contains("Internal"));
        assert!(text.contains("8000/tcp"));
        assert!(text.contains("Compose"));
        assert!(text.contains("telemetry-talk"));
        assert!(text.contains("entrypoint.sh php artisan serve"));
    }

    #[test]
    fn no_command_removes_command_from_human_output() {
        let text = to_human(&sample(), true);
        assert!(!text.contains("Command"));
        assert!(text.contains("Working dir"));
    }

    #[test]
    fn no_command_also_removes_the_container_command() {
        let text = to_human(&docker_sample(), true);
        assert!(text.contains("telemetry-demo-app"));
        assert!(!text.contains("Command"));

        let json = to_json(&docker_sample(), true).unwrap();
        assert!(json.contains("\"container\""));
        assert!(!json.contains("\"command\""));
    }

    #[test]
    fn colors_are_opt_in() {
        assert!(!to_human(&sample(), false).contains('\x1b'));
        assert!(to_human_colored(&sample(), false, true).contains("\x1b["));
    }

    #[test]
    fn no_command_removes_command_from_json() {
        let json = to_json(&sample(), true).unwrap();
        assert!(!json.contains("\"argv\""));
        assert!(!json.contains("\"command\""));
        assert!(json.contains("\"pid\": 42"));
    }

    #[test]
    fn container_metadata_is_optional_and_includes_the_command_by_default() {
        assert!(!to_json(&sample(), false).unwrap().contains("\"container\""));

        let json = to_json(&docker_sample(), false).unwrap();
        assert!(json.contains("\"container\""));
        assert!(json.contains("entrypoint.sh php artisan serve"));
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
                container: None,
            }],
        );
        let text = to_human(&report, false);
        assert!(text.contains("PID"));
        assert!(text.contains('?'));
        assert!(text.contains("unknown"));
        assert!(text.contains("Note"));
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
