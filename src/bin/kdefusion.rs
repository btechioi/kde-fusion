//! `kdefusion`: launches the Kirigami user interface.
//!
//! KDE Fusion deliberately reuses the Qt 6 QML runtime that Plasma already
//! ships instead of linking Qt at build time.  This keeps the Rust build free
//! of CMake, Ninja, qmake and C++ toolchain requirements.

use anyhow::{bail, Context, Result};
use kdefusion::{QML_DIR_ENV, QML_RUNTIME_ENV, VERSION};
use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

fn main() {
    if let Err(err) = run() {
        eprintln!("kdefusion: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("KDE Fusion {VERSION}");
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "Usage: kdefusion [--smoke-test] [--qml-runtime PATH] [--qml-dir PATH] [--api-base URL]\n\n\
             Options:\n  \
             --smoke-test        Load the UI and exit (used by packaging tests)\n  \
             --qml-runtime PATH  Use a specific qml runtime binary\n  \
             --qml-dir PATH      Use a specific QML source directory\n  \
             --api-base URL      Daemon API base URL (default http://127.0.0.1:47812)\n  \
             --help              Show this help\n  \
             --version           Show the version\n\n\
             Environment:\n  \
             {QML_RUNTIME_ENV}  qml runtime binary\n  \
             {QML_DIR_ENV}      QML source directory\n  \
             KDEFUSION_API      daemon API base URL\n  \
             KDEFUSION_SMOKE_TEST=1  same as --smoke-test"
        );
        return Ok(());
    }

    let smoke_test = args.iter().any(|arg| arg == "--smoke-test")
        || std::env::var("KDEFUSION_SMOKE_TEST").as_deref() == Ok("1");

    let runtime = arg_value(&args, "--qml-runtime")
        .or_else(|| std::env::var(QML_RUNTIME_ENV).ok().map(PathBuf::from))
        .or_else(find_runtime)
        .context(format!(
            "no Qt 6 QML runtime found. {}",
            kdefusion::platform::qml_runtime_hint()
        ))?;

    let qml_dir = arg_value(&args, "--qml-dir")
        .or_else(|| std::env::var(QML_DIR_ENV).ok().map(PathBuf::from))
        .or_else(find_qml_dir)
        .context(
            "could not find the KDE Fusion QML directory. Install kdefusion or set KDEFUSION_QML_DIR.",
        )?;

    let main_qml = qml_dir.join("Main.qml");
    if !main_qml.is_file() {
        bail!("{} is missing", main_qml.display());
    }

    let api_base = arg_value(&args, "--api-base")
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(api_base);

    ensure_daemon(&api_base);

    let mut command = Command::new("sh");
    // Launch the QML runtime with argv[0] set to "kdefusion" so Qt derives
    // the application name (and thus the Wayland app id / taskbar icon) from
    // it instead of the runtime binary name ("qml").
    command
        .arg("-c")
        .arg("exec -a kdefusion \"$0\" \"$@\"")
        .arg(&runtime);
    // Let Qt pick the platform plugin.  On a Plasma Wayland session this will
    // be the Wayland plugin; on X11 it will be xcb.  A user can override it with
    // QT_QPA_PLATFORM as usual.
    command
        .arg(&main_qml)
        .current_dir(&qml_dir)
        .env("KDEFUSION_QML_DIR", &qml_dir)
        .env("KDEFUSION_API", &api_base)
        // QML cannot read environment variables directly; pass the API base
        // through as an application argument too.  The `--` separator keeps the
        // Qt QML runtime from trying to load these flags as files.
        .arg("--")
        .arg(format!("--api-base={api_base}"))
        .env(
            "QT_QUICK_CONTROLS_STYLE",
            std::env::var("QT_QUICK_CONTROLS_STYLE").unwrap_or_else(|_| "org.kde.desktop".into()),
        )
        // Load the KDE platform theme so the Qt palette, fonts and the
        // Kirigami color scheme follow System Settings → Colors.  When the
        // frontend is started from the systemd unit, this is not inherited
        // from the Plasma session automatically.
        .env(
            "QT_QPA_PLATFORMTHEME",
            std::env::var("QT_QPA_PLATFORMTHEME").unwrap_or_else(|_| "kde".into()),
        )
        .env(
            "QT_QUICK_CONTROLS_MOBILE",
            std::env::var("QT_QUICK_CONTROLS_MOBILE").unwrap_or_else(|_| "false".into()),
        );
    if smoke_test {
        command.env("KDEFUSION_SMOKE_TEST", "1");
        command.arg("--smoke-test");
        if std::env::var_os("QT_QPA_PLATFORM").is_none() {
            command.env("QT_QPA_PLATFORM", "offscreen");
        }
    }
    // Pass through any options meant for the QML file (currently only the
    // smoke test flag, but this keeps the launcher future proof).
    for arg in &args {
        if !matches!(
            arg.as_str(),
            "--smoke-test"
                | "--qml-runtime"
                | "--qml-dir"
                | "--api-base"
                | "--version"
                | "-V"
                | "--help"
                | "-h"
        ) && !arg.starts_with("--qml-runtime=")
            && !arg.starts_with("--qml-dir=")
            && !arg.starts_with("--api-base=")
        {
            command.arg(arg);
        }
    }

    tracing_stub(&runtime, &main_qml, smoke_test);
    let status = command
        .stdin(Stdio::null())
        .status()
        .with_context(|| format!("run {}", runtime.display()))?;
    if !status.success() {
        bail!("Qt QML runtime exited with {status}");
    }
    Ok(())
}

fn tracing_stub(runtime: &Path, main_qml: &Path, smoke_test: bool) {
    if std::env::var_os("KDEFUSION_DEBUG").is_some() {
        eprintln!(
            "kdefusion: runtime={} qml={} smoke_test={smoke_test}",
            runtime.display(),
            main_qml.display()
        );
    }
}

fn arg_value(args: &[String], name: &str) -> Option<PathBuf> {
    for (index, arg) in args.iter().enumerate() {
        if arg == name {
            return args.get(index + 1).map(PathBuf::from);
        }
        if let Some(value) = arg.strip_prefix(&format!("{name}=")) {
            return Some(PathBuf::from(value));
        }
    }
    None
}

fn api_base() -> String {
    std::env::var("KDEFUSION_API").unwrap_or_else(|_| "http://127.0.0.1:47812".to_string())
}

fn find_runtime() -> Option<PathBuf> {
    let candidates = [
        "/usr/lib/qt6/bin/qml",
        "/usr/lib64/qt6/bin/qml",
        "/usr/lib/x86_64-linux-gnu/qt6/bin/qml",
        "/usr/lib/aarch64-linux-gnu/qt6/bin/qml",
        "/usr/libexec/qt6/qml",
        "/usr/bin/qml6",
        "/usr/bin/qml-qt6",
        "/usr/bin/qml",
    ];
    if let Some(found) = candidates
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
    {
        return Some(found);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .flat_map(|dir| {
                ["qml6", "qml-qt6", "qml"]
                    .into_iter()
                    .map(move |name| dir.join(name))
            })
            .find(|path| path.is_file())
    })
}

fn find_qml_dir() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // Installed location.
    candidates.push(PathBuf::from("/usr/share/kdefusion/qml"));
    candidates.push(PathBuf::from("/usr/local/share/kdefusion/qml"));

    // Development tree: relative to the executable (target/debug/kdefusion).
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().skip(1).take(5) {
            candidates.push(ancestor.join("qml"));
            candidates.push(ancestor.join("share/kdefusion/qml"));
        }
    }

    // Current directory and its parents, for `cargo run` from the repo root.
    if let Ok(cwd) = std::env::current_dir() {
        for ancestor in cwd.ancestors().take(4) {
            candidates.push(ancestor.join("qml"));
        }
    }

    candidates
        .into_iter()
        .find(|dir| dir.join("Main.qml").is_file())
}

fn ensure_daemon(api_base: &str) {
    if daemon_reachable(api_base) {
        return;
    }
    let _ = Command::new("systemctl")
        .args(["--user", "start", "kdefusiond"])
        .status();
    for _ in 0..20 {
        if daemon_reachable(api_base) {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    if daemon_reachable(api_base) {
        return;
    }
    if let Some(binary) = find_daemon_binary() {
        let _ = Command::new(binary)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        for _ in 0..40 {
            if daemon_reachable(api_base) {
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

fn daemon_reachable(api_base: &str) -> bool {
    let (host, port) = parse_api_base(api_base);
    let address = format!("{host}:{port}");
    if let Ok(mut addresses) = address.to_socket_addrs() {
        if let Some(socket_address) = addresses.next() {
            return std::net::TcpStream::connect_timeout(&socket_address, Duration::from_millis(300))
                .is_ok();
        }
    }
    false
}

fn parse_api_base(api_base: &str) -> (String, u16) {
    let mut value = api_base.trim();
    if let Some(rest) = value.strip_prefix("http://") {
        value = rest;
    } else if let Some(rest) = value.strip_prefix("https://") {
        value = rest;
    }
    if let Some(index) = value.find('/') {
        value = &value[..index];
    }
    if let Some((host, port)) = value.rsplit_once(':') {
        if let Ok(port) = port.parse::<u16>() {
            return (host.to_string(), port);
        }
    }
    (value.to_string(), 47812)
}

fn find_daemon_binary() -> Option<PathBuf> {
    if let Ok(current) = std::env::current_exe() {
        if let Some(parent) = current.parent() {
            let candidate = parent.join("kdefusiond");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    for candidate in ["/usr/bin/kdefusiond", "/usr/local/bin/kdefusiond"] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let path = PathBuf::from(home).join(".local/bin/kdefusiond");
        if path.is_file() {
            return Some(path);
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join("kdefusiond"))
            .find(|path| path.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_flag_values() {
        let args = vec![
            "--qml-dir".to_string(),
            "/tmp/qml".to_string(),
            "--smoke-test".to_string(),
        ];
        assert_eq!(
            arg_value(&args, "--qml-dir"),
            Some(PathBuf::from("/tmp/qml"))
        );
        assert_eq!(arg_value(&args, "--missing"), None);
    }

    #[test]
    fn api_base_is_loopback_by_default() {
        if std::env::var_os("KDEFUSION_API").is_none() {
            assert_eq!(api_base(), "http://127.0.0.1:47812");
        }
    }
}
