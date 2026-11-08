//! Clipboard helpers for bidirectional sync between host and viewer.

use std::process::Stdio;
use std::sync::Mutex;

pub static LAST_CLIPBOARD: Mutex<String> = Mutex::new(String::new());

pub async fn get() -> Option<String> {
    let out = tokio::process::Command::new("wl-paste")
        .arg("--no-newline")
        .stdin(Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

pub async fn set(text: &str) {
    use tokio::io::AsyncWriteExt;
    let mut child = match tokio::process::Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return,
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    let _ = child.wait().await;
}

pub fn take_last() -> String {
    LAST_CLIPBOARD.lock().unwrap().clone()
}

pub fn set_last(text: String) {
    *LAST_CLIPBOARD.lock().unwrap() = text;
}