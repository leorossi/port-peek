//! macOS backend.
//!
//! Everything here comes from the kernel:
//!
//! - `proc_listallpids` / `proc_pidinfo(PROC_PIDLISTFDS)` to enumerate processes and fds
//! - `proc_pidfdinfo(PROC_PIDFDSOCKETINFO)` for socket state, address, and port
//! - `proc_pidpath` for the executable path
//! - `proc_pidinfo(PROC_PIDVNODEPATHINFO)` for the working directory
//! - `sysctl(KERN_PROCARGS2)` for the argument vector
//!
//! The layout structs mirror the public SDK headers in `sys/proc_info.h`. The
//! kernel interface itself is documented as private by Apple, but it is the same
//! interface `lsof` and `ps` use and has been stable for many releases.

use std::ffi::CStr;
use std::io;
use std::mem::size_of;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::os::raw::{c_char, c_int, c_void};
use std::path::PathBuf;
use std::ptr;

use crate::{
    Error, Result,
    model::{Listener, Protocol, shell_join},
};

const PROC_PIDLISTFDS: c_int = 1;
const PROC_PIDTBSDINFO: c_int = 3;
const PROC_PIDFDSOCKETINFO: c_int = 3;
const PROC_PIDVNODEPATHINFO: c_int = 9;

const PROX_FDTYPE_SOCKET: u32 = 2;

const SOCKINFO_IN: c_int = 1;
const SOCKINFO_TCP: c_int = 2;

const TSI_S_LISTEN: c_int = 1;

const INI_IPV6: u8 = 0x2;

const MAXPATHLEN: usize = 1024;
const PROC_PIDPATHINFO_MAXSIZE: usize = 4096;

const KERN_PROCARGS2: c_int = 49;
const MAX_ARGV_BYTES: usize = 16 * 1024 * 1024;

// Size of `union` in `struct socket_info` from the current macOS SDK. The
// exact size only decides how many bytes the kernel writes; we never read past
// the members we define.
const SOCKET_INFO_PROTO_SIZE: usize = 528;

unsafe extern "C" {
    fn proc_listallpids(buffer: *mut c_void, buffersize: c_int) -> c_int;
    fn proc_pidinfo(
        pid: c_int,
        flavor: c_int,
        arg: u64,
        buffer: *mut c_void,
        buffersize: c_int,
    ) -> c_int;
    fn proc_pidfdinfo(
        pid: c_int,
        fd: c_int,
        flavor: c_int,
        buffer: *mut c_void,
        buffersize: c_int,
    ) -> c_int;
    fn proc_pidpath(pid: c_int, buffer: *mut c_void, buffersize: u32) -> c_int;
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProcFdInfo {
    proc_fd: c_int,
    proc_fdtype: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ProcBsdInfo {
    pbi_flags: u32,
    pbi_status: u32,
    pbi_xstatus: u32,
    pbi_pid: u32,
    pbi_ppid: u32,
    pbi_uid: libc::uid_t,
    pbi_gid: libc::gid_t,
    pbi_ruid: libc::uid_t,
    pbi_rgid: libc::gid_t,
    pbi_svuid: libc::uid_t,
    pbi_svgid: libc::gid_t,
    rfu_1: u32,
    pbi_comm: [c_char; 16],
    pbi_name: [c_char; 32],
    pbi_nfiles: u32,
    pbi_pgid: u32,
    pbi_pjobc: u32,
    e_tdev: u32,
    e_tpgid: u32,
    pbi_nice: i32,
    pbi_start_tvsec: u64,
    pbi_start_tvusec: u64,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Clone, Copy, Default)]
struct VInfoStat {
    vst_dev: u32,
    vst_mode: u16,
    vst_nlink: u16,
    vst_ino: u64,
    vst_uid: libc::uid_t,
    vst_gid: libc::gid_t,
    vst_atime: i64,
    vst_atimensec: i64,
    vst_mtime: i64,
    vst_mtimensec: i64,
    vst_ctime: i64,
    vst_ctimensec: i64,
    vst_birthtime: i64,
    vst_birthtimensec: i64,
    vst_size: i64,
    vst_blocks: i64,
    vst_blksize: i32,
    vst_flags: u32,
    vst_gen: u32,
    vst_rdev: u32,
    vst_qspare: [i64; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProcFileInfo {
    pfi_openflags: u32,
    pfi_status: u32,
    pfi_offset: i64,
    pfi_type: i32,
    rfu_1: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SockBufInfo {
    sbi_cc: u32,
    sbi_hiwat: u32,
    sbi_mbcnt: u32,
    sbi_mbmax: u32,
    sbi_lowat: u32,
    sbi_flags: i16,
    sbi_timeo: i16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct In4In6Addr {
    i46a_pad32: [u32; 3],
    i46a_addr4: libc::in_addr,
}

#[repr(C)]
#[derive(Clone, Copy)]
union InSiAddr {
    ina_46: In4In6Addr,
    ina_6: libc::in6_addr,
}

impl Default for InSiAddr {
    fn default() -> Self {
        Self {
            ina_46: In4In6Addr::default(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InSiV4 {
    in4_tos: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InSiV6 {
    in6_hlim: u8,
    in6_cksum: i32,
    in6_ifindex: u16,
    in6_hops: i16,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Clone, Copy, Default)]
struct InSockInfo {
    insi_fport: c_int,
    insi_lport: c_int,
    insi_gencnt: u64,
    insi_flags: u32,
    insi_flow: u32,
    insi_vflag: u8,
    insi_ip_ttl: u8,
    rfu_1: u32,
    insi_faddr: InSiAddr,
    insi_laddr: InSiAddr,
    insi_v4: InSiV4,
    insi_v6: InSiV6,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Clone, Copy, Default)]
struct TcpSockInfo {
    tcpsi_ini: InSockInfo,
    tcpsi_state: c_int,
    tcpsi_timer: [c_int; 4],
    tcpsi_mss: c_int,
    tcpsi_flags: u32,
    rfu_1: u32,
    tcpsi_tp: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
union SocketInfoProto {
    bytes: [u8; SOCKET_INFO_PROTO_SIZE],
    _align: u64,
}

impl Default for SocketInfoProto {
    fn default() -> Self {
        Self {
            bytes: [0; SOCKET_INFO_PROTO_SIZE],
        }
    }
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Clone, Copy, Default)]
struct SocketInfo {
    soi_stat: VInfoStat,
    soi_so: u64,
    soi_pcb: u64,
    soi_type: c_int,
    soi_protocol: c_int,
    soi_family: c_int,
    soi_options: i16,
    soi_linger: i16,
    soi_state: i16,
    soi_qlen: i16,
    soi_incqlen: i16,
    soi_qlimit: i16,
    soi_timeo: i16,
    soi_error: u16,
    soi_oobmark: u32,
    soi_rcv: SockBufInfo,
    soi_snd: SockBufInfo,
    soi_kind: c_int,
    rfu_1: u32,
    soi_proto: SocketInfoProto,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SocketFDInfo {
    pfi: ProcFileInfo,
    psi: SocketInfo,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct VnodeInfo {
    vi_stat: VInfoStat,
    vi_type: c_int,
    vi_pad: c_int,
    vi_fsid: libc::fsid_t,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct VnodeInfoPath {
    vip_vi: VnodeInfo,
    vip_path: [c_char; MAXPATHLEN],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ProcVnodePathInfo {
    pvi_cdir: VnodeInfoPath,
    pvi_rdir: VnodeInfoPath,
}

struct ProcessMeta {
    name: Option<String>,
    exe: Option<PathBuf>,
    argv: Option<Vec<String>>,
    cwd: Option<PathBuf>,
}

impl ProcessMeta {
    fn for_pid(pid: c_int, bsd: &ProcBsdInfo) -> Self {
        let exe = read_exe(pid);
        let name = exe
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .or_else(|| read_comm(bsd));

        Self {
            name,
            exe,
            argv: read_argv(pid).filter(|argv| !argv.is_empty()),
            cwd: read_cwd(pid),
        }
    }
}

/// Find TCP or UDP listeners on `port`.
pub(super) fn find_listeners(port: u16, protocol: Protocol) -> Result<Vec<Listener>> {
    let pids = list_pids()?;
    let mut listeners = Vec::new();

    for pid in pids {
        if pid <= 0 {
            continue;
        }

        let Some(bsd) = read_bsd_info(pid) else {
            continue;
        };
        let fds = list_fds(pid, bsd.pbi_nfiles as usize);
        let mut meta: Option<ProcessMeta> = None;

        for fd in fds {
            if fd.proc_fdtype != PROX_FDTYPE_SOCKET {
                continue;
            }

            let Some(socket) = read_socket(pid, fd.proc_fd) else {
                continue;
            };
            let Some((ip, socket_port)) = binding_from_socket(&socket, protocol) else {
                continue;
            };
            if socket_port != port {
                continue;
            }

            if meta.is_none() {
                meta = Some(ProcessMeta::for_pid(pid, &bsd));
            }
            let meta = meta.as_ref().expect("meta was just initialised");

            listeners.push(Listener {
                pid: Some(pid as u32),
                process: meta.name.clone(),
                exe: meta.exe.clone(),
                argv: meta.argv.clone(),
                command: meta.argv.as_ref().map(|argv| shell_join(argv)),
                cwd: meta.cwd.clone(),
                protocol,
                ip,
                port: socket_port,
            });
        }
    }

    Ok(listeners)
}

fn list_pids() -> Result<Vec<c_int>> {
    let count = unsafe { proc_listallpids(ptr::null_mut(), 0) };
    if count < 0 {
        return Err(backend_error(
            "proc_listallpids",
            io::Error::last_os_error(),
        ));
    }
    if count == 0 {
        return Ok(Vec::new());
    }

    let mut pids = vec![0 as c_int; count as usize + 64];
    loop {
        let ret = unsafe {
            proc_listallpids(
                pids.as_mut_ptr().cast::<c_void>(),
                (pids.len() * size_of::<c_int>()) as c_int,
            )
        };
        if ret < 0 {
            return Err(backend_error(
                "proc_listallpids",
                io::Error::last_os_error(),
            ));
        }
        let count = ret as usize;
        if count < pids.len() {
            pids.truncate(count);
            return Ok(pids);
        }
        pids.resize(count + 64, 0);
    }
}

fn read_bsd_info(pid: c_int) -> Option<ProcBsdInfo> {
    let mut info: ProcBsdInfo = unsafe { std::mem::zeroed() };
    let ret = unsafe {
        proc_pidinfo(
            pid,
            PROC_PIDTBSDINFO,
            0,
            (&raw mut info).cast::<c_void>(),
            size_of::<ProcBsdInfo>() as c_int,
        )
    };
    (ret as usize >= size_of::<ProcBsdInfo>()).then_some(info)
}

fn list_fds(pid: c_int, nfiles: usize) -> Vec<ProcFdInfo> {
    if nfiles == 0 {
        return Vec::new();
    }

    // A little headroom: the process may open a descriptor between the two
    // calls. If it opened many more, we may miss some; the next lookup will
    // catch them.
    let capacity = nfiles.saturating_add(16).min(1_000_000);
    let mut fds = vec![ProcFdInfo::default(); capacity];
    let ret = unsafe {
        proc_pidinfo(
            pid,
            PROC_PIDLISTFDS,
            0,
            fds.as_mut_ptr().cast::<c_void>(),
            (capacity * size_of::<ProcFdInfo>()) as c_int,
        )
    };
    if ret <= 0 {
        return Vec::new();
    }

    let count = (ret as usize / size_of::<ProcFdInfo>()).min(capacity);
    fds.truncate(count);
    fds
}

fn read_socket(pid: c_int, fd: c_int) -> Option<SocketFDInfo> {
    let mut info: SocketFDInfo = unsafe { std::mem::zeroed() };
    let ret = unsafe {
        proc_pidfdinfo(
            pid,
            fd,
            PROC_PIDFDSOCKETINFO,
            (&raw mut info).cast::<c_void>(),
            size_of::<SocketFDInfo>() as c_int,
        )
    };
    (ret as usize >= size_of::<SocketFDInfo>()).then_some(info)
}

fn binding_from_socket(socket: &SocketFDInfo, protocol: Protocol) -> Option<(IpAddr, u16)> {
    match protocol {
        Protocol::Tcp => {
            if socket.psi.soi_kind != SOCKINFO_TCP {
                return None;
            }
            let tcp = unsafe { &*ptr::addr_of!(socket.psi.soi_proto).cast::<TcpSockInfo>() };
            if tcp.tcpsi_state != TSI_S_LISTEN {
                return None;
            }
            binding_from_in(&tcp.tcpsi_ini)
        }
        Protocol::Udp => {
            if socket.psi.soi_kind != SOCKINFO_IN || socket.psi.soi_protocol != libc::IPPROTO_UDP {
                return None;
            }
            let info = unsafe { &*ptr::addr_of!(socket.psi.soi_proto).cast::<InSockInfo>() };
            // A connected UDP socket has a foreign port and is a client, not a
            // server. Match Linux's unconnected-only behaviour.
            if info.insi_fport != 0 {
                return None;
            }
            binding_from_in(info)
        }
    }
}

fn binding_from_in(info: &InSockInfo) -> Option<(IpAddr, u16)> {
    let port = u16::from_be(info.insi_lport as u16);
    if port == 0 {
        return None;
    }

    let ip = if info.insi_vflag & INI_IPV6 != 0 {
        let address = unsafe { info.insi_laddr.ina_6 };
        IpAddr::V6(Ipv6Addr::from(address.s6_addr))
    } else {
        let address = unsafe { info.insi_laddr.ina_46.i46a_addr4 };
        IpAddr::V4(Ipv4Addr::from(u32::from_be(address.s_addr)))
    };

    Some((ip, port))
}

fn read_exe(pid: c_int) -> Option<PathBuf> {
    let mut buffer = [0 as c_char; PROC_PIDPATHINFO_MAXSIZE];
    let ret = unsafe {
        proc_pidpath(
            pid,
            buffer.as_mut_ptr().cast::<c_void>(),
            buffer.len() as u32,
        )
    };
    if ret <= 0 {
        return None;
    }

    let path = unsafe { CStr::from_ptr(buffer.as_ptr()) };
    let path = path.to_string_lossy();
    (!path.is_empty()).then(|| PathBuf::from(path.into_owned()))
}

fn read_comm(bsd: &ProcBsdInfo) -> Option<String> {
    let bytes = unsafe {
        std::slice::from_raw_parts(bsd.pbi_comm.as_ptr().cast::<u8>(), bsd.pbi_comm.len())
    };
    let name = CStr::from_bytes_until_nul(bytes).ok()?.to_string_lossy();
    (!name.is_empty()).then(|| name.into_owned())
}

fn read_cwd(pid: c_int) -> Option<PathBuf> {
    let mut info: ProcVnodePathInfo = unsafe { std::mem::zeroed() };
    let ret = unsafe {
        proc_pidinfo(
            pid,
            PROC_PIDVNODEPATHINFO,
            0,
            (&raw mut info).cast::<c_void>(),
            size_of::<ProcVnodePathInfo>() as c_int,
        )
    };
    if (ret as usize) < size_of::<ProcVnodePathInfo>() {
        return None;
    }

    let path = unsafe { CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr()) };
    let path = path.to_string_lossy();
    (!path.is_empty()).then(|| PathBuf::from(path.into_owned()))
}

fn read_argv(pid: c_int) -> Option<Vec<String>> {
    let mut mib = [libc::CTL_KERN, KERN_PROCARGS2, pid];
    let mut size = 0usize;

    let ret = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            ptr::null_mut(),
            &raw mut size,
            ptr::null_mut(),
            0,
        )
    };
    if ret != 0 || size < size_of::<c_int>() || size > MAX_ARGV_BYTES {
        return None;
    }

    let mut buffer = vec![0u8; size];
    let ret = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            buffer.as_mut_ptr().cast::<c_void>(),
            &raw mut size,
            ptr::null_mut(),
            0,
        )
    };
    if ret != 0 {
        return None;
    }
    buffer.truncate(size);

    parse_argv(&buffer)
}

fn parse_argv(buffer: &[u8]) -> Option<Vec<String>> {
    if buffer.len() < size_of::<c_int>() {
        return None;
    }

    let argc = i32::from_ne_bytes(buffer[..4].try_into().ok()?);
    if argc < 0 {
        return None;
    }
    let argc = argc as usize;
    let mut offset = size_of::<c_int>();

    // Skip the executable path.
    offset += buffer.get(offset..)?.iter().position(|byte| *byte == 0)? + 1;
    // Skip alignment padding between the exec path and argv[0].
    while buffer.get(offset) == Some(&0) {
        offset += 1;
    }

    let mut argv = Vec::with_capacity(argc.min(64));
    for _ in 0..argc {
        if offset >= buffer.len() {
            break;
        }
        let end = offset + buffer[offset..].iter().position(|byte| *byte == 0)?;
        argv.push(String::from_utf8_lossy(&buffer[offset..end]).into_owned());
        offset = end + 1;
    }

    Some(argv)
}

fn backend_error(operation: &str, error: io::Error) -> Error {
    Error::Backend(format!("{operation}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_argv(args: &[&str]) -> Vec<u8> {
        let mut buffer = (args.len() as i32).to_ne_bytes().to_vec();
        buffer.extend_from_slice(b"/usr/bin/test\0");
        buffer.extend_from_slice(&[0, 0, 0]); // alignment padding
        for arg in args {
            buffer.extend_from_slice(arg.as_bytes());
            buffer.push(0);
        }
        buffer
    }

    #[test]
    fn parses_argv_with_spaces_and_quotes() {
        let buffer = encode_argv(&["node", "server.js", "a b", "it's", ""]);
        assert_eq!(
            parse_argv(&buffer).unwrap(),
            vec!["node", "server.js", "a b", "it's", ""]
        );
    }

    #[test]
    fn rejects_truncated_argv() {
        assert!(parse_argv(&[]).is_none());
        assert!(parse_argv(&[1, 0, 0, 0]).is_none());
        assert!(parse_argv(&[0xff, 0xff, 0xff, 0xff, b'/', 0, 0, 0]).is_none());
    }

    #[test]
    fn ffi_layouts_match_the_sdk() {
        assert_eq!(size_of::<ProcFdInfo>(), 8);
        assert_eq!(size_of::<ProcBsdInfo>(), 136);
        assert_eq!(size_of::<VInfoStat>(), 136);
        assert_eq!(size_of::<ProcFileInfo>(), 24);
        assert_eq!(size_of::<SockBufInfo>(), 24);
        assert_eq!(size_of::<InSockInfo>(), 80);
        assert_eq!(size_of::<TcpSockInfo>(), 120);
        assert_eq!(size_of::<SocketInfoProto>(), 528);
        assert_eq!(size_of::<SocketInfo>(), 768);
        assert_eq!(size_of::<SocketFDInfo>(), 792);
        assert_eq!(size_of::<ProcVnodePathInfo>(), 2352);
    }

    #[test]
    fn decodes_ipv4_and_ipv6_ports() {
        let mut info = InSockInfo {
            insi_vflag: 0x1,
            ..InSockInfo::default()
        };
        info.insi_lport = i32::from(8080u16.to_be());
        info.insi_laddr = InSiAddr {
            ina_46: In4In6Addr {
                i46a_pad32: [0; 3],
                i46a_addr4: libc::in_addr {
                    s_addr: u32::from_ne_bytes([127, 0, 0, 1]),
                },
            },
        };
        assert_eq!(
            binding_from_in(&info).unwrap(),
            ("127.0.0.1".parse().unwrap(), 8080)
        );

        info.insi_vflag = INI_IPV6;
        info.insi_laddr = InSiAddr {
            ina_6: libc::in6_addr {
                s6_addr: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
            },
        };
        assert_eq!(
            binding_from_in(&info).unwrap(),
            ("::1".parse().unwrap(), 8080)
        );
    }
}
