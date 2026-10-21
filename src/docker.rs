//! Optional Docker container metadata.
//!
//! Docker Desktop on macOS and `docker-proxy` on Linux own the host socket for
//! a published container port. The kernel therefore attributes the listener to
//! the forwarder, not to the process inside the container. When a listener
//! looks like a Docker forwarder, ask the local Docker Engine for its published
//! ports and attach the matching container's metadata.
//!
//! This uses the Docker Engine API over a Unix socket. It does not execute or
//! parse the output of `docker`, `lsof`, `netstat`, or `ss`. The lookup is
//! best-effort: an unavailable socket or an API error simply leaves the normal
//! process details in place.

use std::collections::BTreeMap;
use std::env;
use std::io::{Read, Write};
use std::net::IpAddr;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::model::{ContainerInfo, Listener, Protocol};

const DOCKER_API_REQUEST: &str = "GET /containers/json?all=0 HTTP/1.1\r\nHost: docker\r\nAccept: application/json\r\nConnection: close\r\n\r\n";
const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(2);

/// Attach container metadata to listeners owned by a Docker forwarder.
pub(crate) fn annotate(listeners: &mut [Listener]) {
    if !listeners.iter().any(looks_like_docker) {
        return;
    }

    let Some(containers) = fetch_containers() else {
        return;
    };

    for listener in listeners {
        if !looks_like_docker(listener) {
            continue;
        }
        let protocol = listener.protocol;
        if let Some((container, published)) = matching_container(&containers, listener) {
            listener.container = Some(container_info(container, published, protocol));
        }
    }
}

/// Whether a host listener is likely to be Docker's port forwarder.
fn looks_like_docker(listener: &Listener) -> bool {
    let process = listener.process.as_deref().unwrap_or_default();
    let exe = listener
        .exe
        .as_ref()
        .map_or_else(String::new, |path| path.to_string_lossy().into_owned());

    process == "com.docker.backend"
        || process.contains("docker-proxy")
        || process.contains("vpnkit")
        || exe.contains("/Docker.app/")
        || exe.contains("docker-proxy")
}

fn fetch_containers() -> Option<Vec<DockerContainer>> {
    for socket in socket_candidates() {
        if let Some(containers) = query_socket(&socket) {
            return Some(containers);
        }
    }
    None
}

fn socket_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(host) = env::var("DOCKER_HOST") {
        if let Some(path) = host.strip_prefix("unix://") {
            push_unique(&mut candidates, PathBuf::from(path));
        }
    }

    push_unique(&mut candidates, PathBuf::from("/var/run/docker.sock"));

    if let Ok(runtime) = env::var("XDG_RUNTIME_DIR") {
        push_unique(&mut candidates, PathBuf::from(runtime).join("docker.sock"));
    }

    if let Ok(home) = env::var("HOME") {
        let home = PathBuf::from(home);
        push_unique(&mut candidates, home.join(".docker/run/docker.sock"));
        push_unique(&mut candidates, home.join(".colima/default/docker.sock"));
    }

    candidates
}

fn push_unique(candidates: &mut Vec<PathBuf>, path: PathBuf) {
    if !path.as_os_str().is_empty() && !candidates.contains(&path) {
        candidates.push(path);
    }
}

fn query_socket(path: &Path) -> Option<Vec<DockerContainer>> {
    let mut stream = UnixStream::connect(path).ok()?;
    stream.set_read_timeout(Some(IO_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(IO_TIMEOUT)).ok()?;
    stream.write_all(DOCKER_API_REQUEST.as_bytes()).ok()?;
    stream.flush().ok()?;

    let mut response = Vec::new();
    let mut limited = stream.take(MAX_RESPONSE_BYTES + 1);
    limited.read_to_end(&mut response).ok()?;
    if response.len() as u64 > MAX_RESPONSE_BYTES {
        return None;
    }

    let body = parse_http_response(&response)?;
    let containers = serde_json::from_str::<Vec<DockerContainer>>(&body).ok()?;
    Some(
        containers
            .into_iter()
            .filter(|container| !container.id.is_empty())
            .collect(),
    )
}

fn parse_http_response(response: &[u8]) -> Option<String> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")?;
    let header = &response[..header_end];
    let body = &response[header_end + 4..];

    let mut lines = std::str::from_utf8(header).ok()?.split("\r\n");
    let status = lines.next()?;
    if status.split_whitespace().nth(1) != Some("200") {
        return None;
    }

    let chunked = lines.any(|line| {
        let Some((name, value)) = line.split_once(':') else {
            return false;
        };
        name.trim().eq_ignore_ascii_case("transfer-encoding")
            && value
                .split(',')
                .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
    });

    if chunked {
        decode_chunked(body).and_then(|body| String::from_utf8(body).ok())
    } else {
        String::from_utf8(body.to_vec()).ok()
    }
}

fn decode_chunked(mut data: &[u8]) -> Option<Vec<u8>> {
    let mut decoded = Vec::new();

    loop {
        let line_end = data.windows(2).position(|window| window == b"\r\n")?;
        let size_line = data[..line_end].split(|byte| *byte == b';').next()?;
        let size_text = std::str::from_utf8(size_line).ok()?.trim();
        let size = usize::from_str_radix(size_text, 16).ok()?;
        data = &data[line_end + 2..];

        if size == 0 {
            return Some(decoded);
        }

        let chunk_end = size.checked_add(2)?;
        if data.len() < chunk_end || &data[size..chunk_end] != b"\r\n" {
            return None;
        }
        decoded.extend_from_slice(&data[..size]);
        data = &data[chunk_end..];
    }
}

fn matching_container<'a>(
    containers: &'a [DockerContainer],
    listener: &Listener,
) -> Option<(&'a DockerContainer, &'a DockerPort)> {
    let mut best: Option<(&DockerContainer, &DockerPort, u8)> = None;

    for container in containers {
        let Some(ports) = &container.ports else {
            continue;
        };
        for published in ports {
            if published.public_port != Some(listener.port)
                || !published
                    .kind
                    .eq_ignore_ascii_case(listener.protocol.as_str())
            {
                continue;
            }
            let Some(rank) = ip_rank(published.ip.as_deref(), listener.ip) else {
                continue;
            };
            if best.is_none_or(|(_, _, best_rank)| rank > best_rank) {
                best = Some((container, published, rank));
            }
        }
    }

    best.map(|(container, published, _)| (container, published))
}

/// Rank a published address against the host listener address.
///
/// An exact address wins over a wildcard. A different specific address means
/// this is a mapping for another interface and should not be attached.
fn ip_rank(mapped: Option<&str>, listener_ip: IpAddr) -> Option<u8> {
    let Some(mapped) = mapped else {
        return Some(1);
    };
    let Ok(mapped) = mapped.parse::<IpAddr>() else {
        return Some(1);
    };
    if mapped == listener_ip {
        Some(2)
    } else if mapped.is_unspecified() {
        Some(1)
    } else {
        None
    }
}

fn container_info(
    container: &DockerContainer,
    published: &DockerPort,
    protocol: Protocol,
) -> ContainerInfo {
    let name = container
        .names
        .as_ref()
        .and_then(|names| {
            names.iter().find_map(|name| {
                let name = name.trim_start_matches('/');
                (!name.is_empty()).then(|| name.to_owned())
            })
        })
        .unwrap_or_else(|| short_id(&container.id));

    let image = if container.image.is_empty() {
        "unknown".to_owned()
    } else {
        container.image.clone()
    };

    let command = container
        .command
        .as_deref()
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(ToOwned::to_owned);

    let compose_project = container
        .labels
        .as_ref()
        .and_then(|labels| labels.get("com.docker.compose.project").cloned())
        .filter(|project| !project.is_empty());

    ContainerInfo {
        id: container.id.clone(),
        name,
        image,
        internal_port: published.private_port,
        protocol,
        command,
        compose_project,
    }
}

fn short_id(id: &str) -> String {
    id.get(..12).unwrap_or(id).to_owned()
}

#[derive(Debug, Deserialize)]
struct DockerContainer {
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "Names", default)]
    names: Option<Vec<String>>,
    #[serde(rename = "Image", default)]
    image: String,
    #[serde(rename = "Command", default)]
    command: Option<String>,
    #[serde(rename = "Ports", default)]
    ports: Option<Vec<DockerPort>>,
    #[serde(rename = "Labels", default)]
    labels: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
struct DockerPort {
    #[serde(rename = "IP", default)]
    ip: Option<String>,
    #[serde(rename = "PrivatePort", default)]
    private_port: u16,
    #[serde(rename = "PublicPort", default)]
    public_port: Option<u16>,
    #[serde(rename = "Type", default)]
    kind: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listener(process: &str, ip: &str, port: u16) -> Listener {
        Listener {
            pid: Some(1),
            process: Some(process.to_owned()),
            exe: None,
            argv: None,
            command: None,
            cwd: None,
            protocol: Protocol::Tcp,
            ip: ip.parse().unwrap(),
            port,
            container: None,
        }
    }

    fn published(ip: &str, public: u16, private: u16, kind: &str) -> DockerPort {
        DockerPort {
            ip: Some(ip.to_owned()),
            private_port: private,
            public_port: Some(public),
            kind: kind.to_owned(),
        }
    }

    fn container(ports: Vec<DockerPort>) -> DockerContainer {
        DockerContainer {
            id: "9f8cac51baba0000000000000000000000000000000000000000000000000000".to_owned(),
            names: Some(vec!["/telemetry-demo-app".to_owned()]),
            image: "telemetry-demo-app".to_owned(),
            command: Some("entrypoint.sh php artisan serve".to_owned()),
            ports: Some(ports),
            labels: Some(BTreeMap::from([(
                "com.docker.compose.project".to_owned(),
                "telemetry-talk".to_owned(),
            )])),
        }
    }

    #[test]
    fn decodes_chunked_bodies() {
        let body = b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(decode_chunked(body).unwrap(), b"Wikipedia");
    }

    #[test]
    fn parses_a_chunked_http_response() {
        let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(parse_http_response(response).as_deref(), Some("Wikipedia"));
    }

    #[test]
    fn parses_a_non_chunked_http_response() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n[]";
        assert_eq!(parse_http_response(response).as_deref(), Some("[]"));
    }

    #[test]
    fn rejects_non_success_responses() {
        let response = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        assert!(parse_http_response(response).is_none());
    }

    #[test]
    fn recognizes_docker_forwarders() {
        assert!(looks_like_docker(&listener(
            "com.docker.backend",
            "::",
            8000
        )));
        assert!(looks_like_docker(&listener(
            "docker-proxy",
            "0.0.0.0",
            8000
        )));
        assert!(!looks_like_docker(&listener("node", "127.0.0.1", 8000)));
    }

    #[test]
    fn matches_a_published_port() {
        let listener = listener("docker-proxy", "::", 8000);
        let containers = vec![container(vec![
            published("0.0.0.0", 8000, 8000, "tcp"),
            published("::", 8000, 8000, "tcp"),
        ])];

        let (matched, port) = matching_container(&containers, &listener).unwrap();
        assert_eq!(matched.id, containers[0].id);
        assert_eq!(port.private_port, 8000);
    }

    #[test]
    fn prefers_an_exact_listener_address() {
        let listener = listener("docker-proxy", "127.0.0.1", 8080);
        let containers = vec![
            container(vec![published("0.0.0.0", 8080, 80, "tcp")]),
            DockerContainer {
                id: "another-container".to_owned(),
                names: Some(vec!["/exact".to_owned()]),
                image: "exact".to_owned(),
                command: None,
                ports: Some(vec![published("127.0.0.1", 8080, 81, "tcp")]),
                labels: None,
            },
        ];

        let (matched, port) = matching_container(&containers, &listener).unwrap();
        assert_eq!(matched.id, "another-container");
        assert_eq!(port.private_port, 81);
    }

    #[test]
    fn ignores_other_protocols() {
        let listener = listener("docker-proxy", "127.0.0.1", 8080);
        let containers = vec![container(vec![published("127.0.0.1", 8080, 80, "udp")])];
        assert!(matching_container(&containers, &listener).is_none());
    }

    #[test]
    fn builds_container_metadata() {
        let published = published("::", 8000, 8000, "tcp");
        let info = container_info(
            &container(vec![published.clone()]),
            &published,
            Protocol::Tcp,
        );

        assert_eq!(info.name, "telemetry-demo-app");
        assert_eq!(info.short_id(), "9f8cac51baba");
        assert_eq!(info.image, "telemetry-demo-app");
        assert_eq!(info.internal_port, 8000);
        assert_eq!(info.protocol, Protocol::Tcp);
        assert_eq!(
            info.command.as_deref(),
            Some("entrypoint.sh php artisan serve")
        );
        assert_eq!(info.compose_project.as_deref(), Some("telemetry-talk"));
    }

    #[test]
    fn accepts_containers_without_ports() {
        let json = r#"[{"Id":"abc","Names":["/app"],"Image":"img","Command":"","Ports":null,"Labels":null}]"#;
        let containers: Vec<DockerContainer> = serde_json::from_str(json).unwrap();
        assert!(containers[0].ports.is_none());
    }
}
