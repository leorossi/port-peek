//! Linux backend.
//!
//! The socket table is read with a `NETLINK_SOCK_DIAG` dump. Each socket's
//! inode is then mapped to a PID by reading `/proc/<pid>/fd` symlinks, and the
//! process metadata (exe, cwd, cmdline) comes from `/proc/<pid>/{exe,cwd,cmdline}`.
//!
//! None of this parses the output of `ss`, `netstat`, or `lsof`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::mem::size_of;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::raw::{c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr;

use crate::{
    Error, Result,
    model::{Listener, Protocol, shell_join},
};

const NETLINK_SOCK_DIAG: c_int = 4;
const SOCK_DIAG_BY_FAMILY: u16 = 20;

const NLM_F_REQUEST: u16 = 0x01;
const NLM_F_DUMP: u16 = 0x300;

const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;

const TCP_CLOSE: u8 = 7;
const TCP_LISTEN: u8 = 10;

const INET_DIAG_MSG_LEN: usize = 72;
const INET_DIAG_INODE_OFFSET: usize = 68;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct NlMsgHdr {
    nlmsg_len: u32,
    nlmsg_type: u16,
    nlmsg_flags: u16,
    nlmsg_seq: u32,
    nlmsg_pid: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InetDiagSockId {
    idiag_sport: u16,
    idiag_dport: u16,
    idiag_src: [u32; 4],
    idiag_dst: [u32; 4],
    idiag_if: u32,
    idiag_cookie: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InetDiagReqV2 {
    sdiag_family: u8,
    sdiag_protocol: u8,
    idiag_ext: u8,
    pad: u8,
    idiag_states: u32,
    id: InetDiagSockId,
}

#[derive(Clone, Copy)]
struct SocketRecord {
    ip: IpAddr,
    port: u16,
    inode: u32,
}

#[derive(Clone)]
struct ProcessMeta {
    name: Option<String>,
    exe: Option<PathBuf>,
    argv: Option<Vec<String>>,
    cwd: Option<PathBuf>,
}

impl ProcessMeta {
    fn for_pid(pid: u32) -> Self {
        let exe = fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .map(strip_deleted);
        let name = exe
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .or_else(|| {
                fs::read_to_string(format!("/proc/{pid}/comm"))
                    .ok()
                    .map(|name| name.trim().to_owned())
                    .filter(|name| !name.is_empty())
            });

        Self {
            name,
            exe,
            argv: read_cmdline(pid),
            cwd: fs::read_link(format!("/proc/{pid}/cwd"))
                .ok()
                .map(strip_deleted),
        }
    }
}

/// Find TCP or UDP listeners on `port`.
pub(super) fn find_listeners(port: u16, protocol: Protocol) -> Result<Vec<Listener>> {
    let mut records = Vec::new();
    let mut errors = Vec::new();
    let mut any_family_succeeded = false;

    for family in [libc::AF_INET, libc::AF_INET6] {
        match query_family(family, protocol, port) {
            Ok(mut found) => {
                any_family_succeeded = true;
                records.append(&mut found);
            }
            Err(error) => errors.push(error.to_string()),
        }
    }

    if !any_family_succeeded && !errors.is_empty() {
        return Err(Error::Backend(errors.join("; ")));
    }

    let mut targets: HashMap<u32, SocketRecord> = HashMap::new();
    for record in records {
        targets.entry(record.inode).or_insert(record);
    }
    if targets.is_empty() {
        return Ok(Vec::new());
    }

    let inodes: HashSet<u32> = targets.keys().copied().collect();
    let pid_by_inode = map_inodes_to_pids(&inodes);
    let mut meta_cache: HashMap<u32, ProcessMeta> = HashMap::new();
    let mut listeners = Vec::with_capacity(targets.len());

    for (inode, record) in targets {
        let pid = pid_by_inode.get(&inode).copied();
        let meta = pid.map(|pid| {
            meta_cache
                .entry(pid)
                .or_insert_with(|| ProcessMeta::for_pid(pid))
                .clone()
        });

        listeners.push(Listener {
            pid,
            process: meta.as_ref().and_then(|meta| meta.name.clone()),
            exe: meta.as_ref().and_then(|meta| meta.exe.clone()),
            argv: meta.as_ref().and_then(|meta| meta.argv.clone()),
            command: meta
                .as_ref()
                .and_then(|meta| meta.argv.as_ref())
                .map(|argv| shell_join(argv)),
            cwd: meta.as_ref().and_then(|meta| meta.cwd.clone()),
            protocol,
            ip: record.ip,
            port: record.port,
        });
    }

    Ok(listeners)
}

fn query_family(family: c_int, protocol: Protocol, port: u16) -> Result<Vec<SocketRecord>> {
    let fd = open_diag_socket()?;
    let states = match protocol {
        Protocol::Tcp => 1u32 << TCP_LISTEN,
        Protocol::Udp => u32::MAX,
    };
    let protocol_number = match protocol {
        Protocol::Tcp => libc::IPPROTO_TCP as u8,
        Protocol::Udp => libc::IPPROTO_UDP as u8,
    };

    let request = build_request(family as u8, protocol_number, states);
    let sent = unsafe {
        libc::send(
            fd.as_raw_fd(),
            request.as_ptr().cast::<c_void>(),
            request.len(),
            0,
        )
    };
    if sent < 0 {
        return Err(backend_error("netlink send", io::Error::last_os_error()));
    }
    if sent as usize != request.len() {
        return Err(Error::Backend(
            "netlink send: short write of the diagnostic request".to_owned(),
        ));
    }

    let mut records = Vec::new();
    let mut buffer = vec![0u8; 64 * 1024];

    loop {
        let received = unsafe {
            libc::recv(
                fd.as_raw_fd(),
                buffer.as_mut_ptr().cast::<c_void>(),
                buffer.len(),
                0,
            )
        };
        if received < 0 {
            return Err(backend_error("netlink recv", io::Error::last_os_error()));
        }
        if received == 0 {
            return Err(Error::Backend("netlink recv: connection closed".to_owned()));
        }

        let received = received as usize;
        let mut offset = 0;
        while offset + size_of::<NlMsgHdr>() <= received {
            let length = u32::from_ne_bytes(
                buffer[offset..offset + 4]
                    .try_into()
                    .expect("slice has four bytes"),
            ) as usize;
            let message_type = u16::from_ne_bytes(
                buffer[offset + 4..offset + 6]
                    .try_into()
                    .expect("slice has two bytes"),
            );

            if length < size_of::<NlMsgHdr>() || offset + length > received {
                break;
            }

            match message_type {
                NLMSG_DONE => return Ok(records),
                NLMSG_ERROR => {
                    let error = i32::from_ne_bytes(
                        buffer[offset + 16..offset + 20]
                            .try_into()
                            .expect("slice has four bytes"),
                    );
                    if error != 0 {
                        return Err(backend_error(
                            "netlink",
                            io::Error::from_raw_os_error(-error),
                        ));
                    }
                }
                SOCK_DIAG_BY_FAMILY => {
                    let payload = &buffer[offset + size_of::<NlMsgHdr>()..offset + length];
                    if let Some(record) = parse_diag(payload, family, protocol, port) {
                        records.push(record);
                    }
                }
                _ => {}
            }

            offset += (length + 3) & !3;
        }
    }
}

fn open_diag_socket() -> Result<OwnedFd> {
    let fd = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            NETLINK_SOCK_DIAG,
        )
    };
    if fd < 0 {
        return Err(backend_error("netlink socket", io::Error::last_os_error()));
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };

    let mut address: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    address.nl_family = libc::AF_NETLINK as libc::sa_family_t;
    address.nl_pid = 0;
    address.nl_groups = 0;
    let bound = unsafe {
        libc::bind(
            fd.as_raw_fd(),
            ptr::from_ref(&address).cast(),
            size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        )
    };
    if bound < 0 {
        return Err(backend_error("netlink bind", io::Error::last_os_error()));
    }

    // A dump normally completes in milliseconds. Do not hang forever if the
    // kernel never sends NLMSG_DONE.
    let timeout = libc::timeval {
        tv_sec: 5,
        tv_usec: 0,
    };
    unsafe {
        let result = libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            ptr::from_ref(&timeout).cast(),
            size_of::<libc::timeval>() as libc::socklen_t,
        );
        if result < 0 {
            return Err(backend_error(
                "netlink setsockopt",
                io::Error::last_os_error(),
            ));
        }
    }

    Ok(fd)
}

fn build_request(family: u8, protocol: u8, states: u32) -> Vec<u8> {
    let header = NlMsgHdr {
        nlmsg_len: (size_of::<NlMsgHdr>() + size_of::<InetDiagReqV2>()) as u32,
        nlmsg_type: SOCK_DIAG_BY_FAMILY,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_DUMP,
        nlmsg_seq: 1,
        nlmsg_pid: 0,
    };
    let request = InetDiagReqV2 {
        sdiag_family: family,
        sdiag_protocol: protocol,
        idiag_ext: 0,
        pad: 0,
        idiag_states: states,
        id: InetDiagSockId::default(),
    };

    let mut buffer = Vec::with_capacity(size_of::<NlMsgHdr>() + size_of::<InetDiagReqV2>());
    buffer.extend_from_slice(as_bytes(&header));
    buffer.extend_from_slice(as_bytes(&request));
    buffer
}

fn parse_diag(
    payload: &[u8],
    family: c_int,
    protocol: Protocol,
    port: u16,
) -> Option<SocketRecord> {
    if payload.len() < INET_DIAG_MSG_LEN {
        return None;
    }

    let state = payload[1];
    let expected = match protocol {
        Protocol::Tcp => TCP_LISTEN,
        Protocol::Udp => TCP_CLOSE,
    };
    if state != expected {
        return None;
    }

    let socket_port = u16::from_be_bytes([payload[4], payload[5]]);
    if socket_port != port {
        return None;
    }

    let address_bytes = &payload[8..24];
    let ip = match family {
        libc::AF_INET => IpAddr::V4(Ipv4Addr::new(
            address_bytes[0],
            address_bytes[1],
            address_bytes[2],
            address_bytes[3],
        )),
        libc::AF_INET6 => {
            let bytes: [u8; 16] = address_bytes.try_into().ok()?;
            IpAddr::V6(Ipv6Addr::from(bytes))
        }
        _ => return None,
    };

    let inode = u32::from_ne_bytes(
        payload[INET_DIAG_INODE_OFFSET..INET_DIAG_INODE_OFFSET + 4]
            .try_into()
            .ok()?,
    );
    if inode == 0 {
        return None;
    }

    Some(SocketRecord {
        ip,
        port: socket_port,
        inode,
    })
}

fn map_inodes_to_pids(inodes: &HashSet<u32>) -> HashMap<u32, u32> {
    let mut pid_by_inode = HashMap::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return pid_by_inode;
    };

    let mut pids: Vec<u32> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_string_lossy().parse().ok())
        .collect();
    // Deterministic choice when a socket was inherited by several processes:
    // the lowest PID wins.
    pids.sort_unstable();

    for pid in pids {
        if pid_by_inode.len() == inodes.len() {
            break;
        }

        let Ok(fds) = fs::read_dir(format!("/proc/{pid}/fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            let Ok(link) = fs::read_link(fd.path()) else {
                continue;
            };
            let Some(inode) = socket_inode(&link) else {
                continue;
            };
            if inodes.contains(&inode) && !pid_by_inode.contains_key(&inode) {
                pid_by_inode.insert(inode, pid);
            }
        }
    }

    pid_by_inode
}

fn socket_inode(path: &Path) -> Option<u32> {
    let text = path.to_str()?;
    text.strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

fn read_cmdline(pid: u32) -> Option<Vec<String>> {
    let mut bytes = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    if bytes.is_empty() {
        return None;
    }

    Some(
        bytes
            .split(|byte| *byte == 0)
            .map(|arg| String::from_utf8_lossy(arg).into_owned())
            .collect(),
    )
}

fn strip_deleted(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(stripped) = text.strip_suffix(" (deleted)") {
        PathBuf::from(stripped)
    } else {
        path
    }
}

fn as_bytes<T>(value: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts(ptr::from_ref(value).cast::<u8>(), size_of::<T>()) }
}

fn backend_error(operation: &str, error: io::Error) -> Error {
    Error::Backend(format!("{operation}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_layout_matches_the_kernel_abi() {
        assert_eq!(size_of::<NlMsgHdr>(), 16);
        assert_eq!(size_of::<InetDiagReqV2>(), 56);
        assert_eq!(size_of::<InetDiagSockId>(), 48);
        assert_eq!(build_request(libc::AF_INET as u8, 6, 1).len(), 72);
    }

    #[test]
    fn parses_an_ipv4_diag_message() {
        let mut payload = [0u8; INET_DIAG_MSG_LEN];
        payload[1] = TCP_LISTEN;
        payload[4..6].copy_from_slice(&8080u16.to_be_bytes());
        payload[8..12].copy_from_slice(&[127, 0, 0, 1]);
        payload[INET_DIAG_INODE_OFFSET..INET_DIAG_INODE_OFFSET + 4]
            .copy_from_slice(&42u32.to_ne_bytes());

        let record = parse_diag(&payload, libc::AF_INET, Protocol::Tcp, 8080).unwrap();
        assert_eq!(record.ip, "127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(record.port, 8080);
        assert_eq!(record.inode, 42);
    }

    #[test]
    fn ignores_non_listening_sockets() {
        let mut payload = [0u8; INET_DIAG_MSG_LEN];
        payload[1] = 1; // TCP_ESTABLISHED
        payload[4..6].copy_from_slice(&8080u16.to_be_bytes());
        assert!(parse_diag(&payload, libc::AF_INET, Protocol::Tcp, 8080).is_none());
    }

    #[test]
    fn only_reports_unconnected_udp_sockets() {
        let mut payload = [0u8; INET_DIAG_MSG_LEN];
        payload[4..6].copy_from_slice(&8080u16.to_be_bytes());
        payload[8..12].copy_from_slice(&[127, 0, 0, 1]);
        payload[INET_DIAG_INODE_OFFSET..INET_DIAG_INODE_OFFSET + 4]
            .copy_from_slice(&42u32.to_ne_bytes());

        payload[1] = TCP_CLOSE;
        assert!(parse_diag(&payload, libc::AF_INET, Protocol::Udp, 8080).is_some());

        payload[1] = 1; // TCP_ESTABLISHED, i.e. a connected UDP client
        assert!(parse_diag(&payload, libc::AF_INET, Protocol::Udp, 8080).is_none());
    }

    #[test]
    fn extracts_socket_inodes() {
        assert_eq!(socket_inode(Path::new("socket:[1234]")), Some(1234));
        assert_eq!(socket_inode(Path::new("/tmp/file")), None);
    }
}
