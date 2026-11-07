//! Small shared helpers. Deliberately dependency-light so the daemon can be
//! built on a machine with no CMake, no Qt development headers and no root.

use anyhow::{bail, Context, Result};
use std::ffi::OsStr;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::process::Command;

pub const APP_DIR: &str = "kdefusion";

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn random_hex(bytes: usize) -> String {
    let mut out = String::with_capacity(bytes * 2);
    for _ in 0..bytes {
        out.push_str(&format!("{:02x}", fastrand::u8(..)));
    }
    out
}

pub fn random_digits(len: usize) -> String {
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        out.push(char::from(b'0' + fastrand::u8(0..10)));
    }
    out
}

pub fn random_pin4() -> String {
    random_digits(4)
}

pub fn random_pair_code() -> String {
    random_digits(6)
}

pub fn random_token() -> String {
    random_hex(32)
}

/// Ask the kernel for an unused TCP port on the loopback interface.
pub fn find_free_port() -> Result<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).context("bind ephemeral port")?;
    let port = listener.local_addr()?.port();
    Ok(port)
}

pub fn is_port_free(port: u16) -> bool {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok()
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| home_dir().join(".config"))
        .join(APP_DIR)
}

pub fn state_dir() -> PathBuf {
    dirs::state_dir()
        .unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(|| home_dir().join(".local/share"))
                .join("state")
        })
        .join(APP_DIR)
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| home_dir().join(".local/share"))
        .join(APP_DIR)
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).with_context(|| format!("create directory {}", path.display()))
}

pub fn atomic_write(path: &Path, contents: &[u8], mode: Option<u32>) -> Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(OsStr::to_str).unwrap_or("file"),
        random_hex(4)
    ));
    {
        let mut f = std::fs::File::create(&tmp)
            .with_context(|| format!("create temporary file {}", tmp.display()))?;
        f.write_all(contents)
            .with_context(|| format!("write {}", tmp.display()))?;
        f.sync_all().ok();
    }
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode)).ok();
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "kde-fusion".to_string())
}

pub fn local_ipv4() -> Option<Ipv4Addr> {
    // Connecting a UDP socket does not send any packets but lets the kernel
    // pick the address it would use for the default route.
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    sock.connect((Ipv4Addr::new(1, 1, 1, 1), 53)).ok()?;
    match sock.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_loopback() => Some(v4),
        _ => None,
    }
}

/// Parse `ip -4 -o addr show scope global` as a fallback / supplemental
/// source of LAN addresses.
pub fn lan_ipv4_addresses() -> Vec<Ipv4Addr> {
    let mut addrs = Vec::new();
    if let Some(ip) = local_ipv4() {
        addrs.push(ip);
    }
    if let Ok(out) = std::process::Command::new("ip")
        .args(["-4", "-o", "addr", "show", "scope", "global"])
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            // 2: wlan0    inet 192.168.1.4/24 brd ...
            if let Some(idx) = line.find(" inet ") {
                let rest = &line[idx + 6..];
                if let Some(cidr) = rest.split_whitespace().next() {
                    if let Some((ip, _)) = cidr.split_once('/') {
                        if let Ok(ip) = ip.parse::<Ipv4Addr>() {
                            if !ip.is_loopback() && !addrs.contains(&ip) {
                                addrs.push(ip);
                            }
                        }
                    }
                }
            }
        }
    }
    addrs
}

pub fn command_exists(name: &str) -> bool {
    if name.contains('/') {
        return Path::new(name).is_file();
    }
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let candidate = dir.join(name);
                candidate.is_file()
            })
        })
        .unwrap_or(false)
}

pub fn command_path(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        return Path::new(name).is_file().then(|| PathBuf::from(name));
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

#[derive(Debug, Clone)]
pub struct CmdOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

pub async fn run(program: &str, args: &[String]) -> Result<CmdOutput> {
    let output = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .await
        .with_context(|| format!("run {program}"))?;
    Ok(CmdOutput {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

pub async fn run_checked(program: &str, args: &[String]) -> Result<CmdOutput> {
    let out = run(program, args).await?;
    if out.status != 0 {
        bail!(
            "{} {} failed (exit {}): {}",
            program,
            args.join(" "),
            out.status,
            first_nonempty(&out.stderr, &out.stdout)
        );
    }
    Ok(out)
}

pub fn first_nonempty<'a>(a: &'a str, b: &'a str) -> &'a str {
    let a = a.trim();
    if a.is_empty() {
        b.trim()
    } else {
        a
    }
}

/// Poll an async predicate until it returns Some, or the timeout expires.
pub async fn poll_until<F, Fut, T>(timeout: Duration, interval: Duration, mut f: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(value) = f().await {
            return Some(value);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(interval).await;
    }
}

pub fn parse_socket_addr(host: &str, port: u16) -> Result<SocketAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, port));
    }
    use std::net::ToSocketAddrs;
    (host, port)
        .to_socket_addrs()
        .context("resolve address")?
        .next()
        .context("no address returned")
}

pub fn sanitize_service_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "KDE Fusion".to_string()
    } else {
        cleaned.chars().take(60).collect()
    }
}

/// Escape a value for a curl config file passed over stdin.
pub fn curl_config_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn format_duration_secs(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

pub fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|v| v.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false)
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

pub fn is_kde() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|v| v.to_ascii_lowercase().contains("kde"))
        .unwrap_or(false)
        || std::env::var_os("KDE_FULL_SESSION").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_values_have_expected_shape() {
        assert_eq!(random_hex(8).len(), 16);
        assert_eq!(random_pin4().len(), 4);
        assert_eq!(random_pair_code().len(), 6);
        assert_eq!(random_token().len(), 64);
        assert!(random_pin4().chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn service_name_is_sanitized() {
        assert_eq!(sanitize_service_name("My PC!"), "My PC-");
        assert_eq!(sanitize_service_name(""), "KDE Fusion");
    }

    #[test]
    fn curl_escaping_works() {
        assert_eq!(curl_config_escape("a\"b\\c"), "a\\\"b\\\\c");
    }
}
