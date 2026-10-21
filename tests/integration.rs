use std::fs;
use std::io::{BufRead, BufReader};
use std::net::{IpAddr, TcpListener, UdpSocket};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use portpeek::{Listener, Protocol, find_listeners};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_portpeek")
}

struct ChildGuard(Child);

impl ChildGuard {
    fn id(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for(port: u16, pid: u32, protocol: Protocol) -> Option<Listener> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let report = find_listeners(port, protocol).expect("lookup failed");
        if let Some(listener) = report
            .listeners
            .into_iter()
            .find(|listener| listener.pid == Some(pid))
        {
            return Some(listener);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn spawn_python_listener(directory: &Path, extra_args: &[&str]) -> (ChildGuard, u16) {
    // Bind an ephemeral port, announce it, then stay alive so the parent can
    // inspect the socket and process metadata.
    let script = r#"
import socket, sys, time
sock = socket.socket()
sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
sock.bind(('127.0.0.1', 0))
sock.listen(1)
print(sock.getsockname()[1], flush=True)
time.sleep(60)
"#;

    let mut child = Command::new("python3")
        .arg("-c")
        .arg(script)
        .args(extra_args)
        .current_dir(directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("python3 is required for the integration test");

    let stdout = child.stdout.take().expect("piped stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read child port");
    let port = line
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("unexpected child output: {line:?}"));

    (ChildGuard(child), port)
}

#[test]
fn finds_the_current_process_tcp_listener() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let pid = std::process::id();

    let report = find_listeners(port, Protocol::Tcp).expect("lookup");
    let found = report
        .listeners
        .iter()
        .find(|listener| listener.pid == Some(pid) && listener.port == port)
        .unwrap_or_else(|| panic!("own listener not found in {report:?}"));

    assert_eq!(found.ip, "127.0.0.1".parse::<IpAddr>().unwrap());
    assert!(found.process.is_some(), "process name missing");
    assert!(found.exe.is_some(), "executable path missing");
    assert!(found.cwd.is_some(), "working directory missing");
    assert!(
        found.argv.as_ref().is_some_and(|argv| !argv.is_empty()),
        "argv missing"
    );
    assert!(found.command.is_some(), "rendered command missing");
}

#[test]
fn finds_an_ipv6_listener_when_available() {
    let Ok(listener) = TcpListener::bind("[::1]:0") else {
        return;
    };
    let port = listener.local_addr().unwrap().port();
    let pid = std::process::id();

    let report = find_listeners(port, Protocol::Tcp).expect("lookup");
    let found = report
        .listeners
        .iter()
        .find(|listener| listener.pid == Some(pid))
        .unwrap_or_else(|| panic!("IPv6 listener not found in {report:?}"));

    assert_eq!(found.ip, "::1".parse::<IpAddr>().unwrap());
    assert_eq!(found.endpoint(), format!("[::1]:{port}"));
}

#[test]
fn finds_a_udp_socket() {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind");
    let port = socket.local_addr().unwrap().port();
    let pid = std::process::id();

    let report = find_listeners(port, Protocol::Udp).expect("lookup");
    let found = report
        .listeners
        .iter()
        .find(|listener| listener.pid == Some(pid))
        .unwrap_or_else(|| panic!("UDP socket not found in {report:?}"));

    assert_eq!(found.ip, "127.0.0.1".parse::<IpAddr>().unwrap());
    assert_eq!(found.port, port);
}

#[test]
fn connected_udp_client_is_not_reported_as_a_listener() {
    let receiver = UdpSocket::bind("127.0.0.1:0").expect("bind receiver");
    let client = UdpSocket::bind("127.0.0.1:0").expect("bind client");
    client
        .connect(receiver.local_addr().unwrap())
        .expect("connect client");
    let port = client.local_addr().unwrap().port();
    let pid = std::process::id();

    let report = find_listeners(port, Protocol::Udp).expect("lookup");
    assert!(
        !report
            .listeners
            .iter()
            .any(|listener| listener.pid == Some(pid) && listener.port == port),
        "connected UDP client was reported as a listener: {report:?}"
    );
}

#[test]
fn reports_the_full_command_line_and_working_directory() {
    let directory = tempfile::tempdir().expect("tempdir");
    let argument_with_spaces = "arg with spaces";
    let argument_with_quote = "quote'inside";

    let (child, port) = spawn_python_listener(
        directory.path(),
        &[argument_with_spaces, argument_with_quote],
    );
    let found = wait_for(port, child.id(), Protocol::Tcp)
        .unwrap_or_else(|| panic!("child listener on {port} not found"));

    let argv = found.argv.as_ref().expect("argv missing");
    assert!(
        argv.iter().any(|arg| arg == argument_with_spaces),
        "argv did not preserve spaces: {argv:?}"
    );
    assert!(
        argv.iter().any(|arg| arg == argument_with_quote),
        "argv did not preserve quotes: {argv:?}"
    );

    let command = found.command.as_ref().expect("command missing");
    assert!(
        command.contains("'arg with spaces'"),
        "command was not shell-escaped: {command}"
    );
    assert!(
        command.contains("'quote'\\''inside'"),
        "single quote was not escaped: {command}"
    );

    let expected_cwd = fs::canonicalize(directory.path()).expect("canonicalize tempdir");
    let actual_cwd = fs::canonicalize(found.cwd.as_ref().expect("cwd missing")).expect("cwd");
    assert_eq!(actual_cwd, expected_cwd);
}

#[test]
fn returns_an_empty_report_for_an_unused_port() {
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let report = find_listeners(port, Protocol::Tcp).expect("lookup");
    assert!(
        report
            .listeners
            .iter()
            .all(|listener| listener.port != port),
        "unexpected listener on unused port {port}: {report:?}"
    );
}

#[test]
fn cli_output_is_plain_when_piped() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();

    let output = Command::new(binary())
        .arg(port.to_string())
        .output()
        .expect("run portpeek");
    assert!(output.status.success());
    assert!(
        !output.stdout.contains(&0x1b),
        "piped output unexpectedly contained ANSI escapes"
    );
}

#[test]
fn cli_exits_one_when_nothing_is_listening() {
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let output = Command::new(binary())
        .arg(port.to_string())
        .output()
        .expect("run portpeek");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("no listener"),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn cli_rejects_an_invalid_port() {
    let output = Command::new(binary())
        .arg("0")
        .output()
        .expect("run portpeek");
    assert_eq!(output.status.code(), Some(2));
    assert!(!output.stderr.is_empty());
}

#[test]
fn cli_json_contains_the_listener_and_respects_no_command() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();

    let output = Command::new(binary())
        .arg(port.to_string())
        .arg("--json")
        .output()
        .expect("run portpeek");
    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON output");
    assert_eq!(json["port"], port);
    assert_eq!(json["protocol"], "tcp");
    let listeners = json["listeners"].as_array().expect("listeners array");
    assert!(
        listeners
            .iter()
            .any(|listener| listener["pid"] == std::process::id()),
        "own listener missing from JSON: {json}"
    );

    let output = Command::new(binary())
        .arg(port.to_string())
        .arg("--json")
        .arg("--no-command")
        .output()
        .expect("run portpeek");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON output");
    for listener in json["listeners"].as_array().expect("listeners array") {
        assert!(listener.get("argv").is_none());
        assert!(listener.get("command").is_none());
    }
}
