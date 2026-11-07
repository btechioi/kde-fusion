//! Small wrapper around `notify-send` so the daemon does not need a D-Bus
//! notification dependency.  KDE Plasma displays these as native notifications.

use tokio::process::Command;

pub async fn send(title: impl AsRef<str>, body: impl AsRef<str>) {
    send_urgency(title, body, "normal").await;
}

pub async fn send_urgency(title: impl AsRef<str>, body: impl AsRef<str>, urgency: &str) {
    let title = title.as_ref().to_string();
    let body = body.as_ref().to_string();
    tracing::info!(target: "notification", "{title}: {body}");
    let expire = if urgency == "critical" { "8000" } else { "5000" };
    let _ = Command::new("notify-send")
        .args([
            "--app-name=KDE Fusion",
            "--icon=kdefusion",
            &format!("--urgency={urgency}"),
            &format!("--expire-time={expire}"),
            "--hint=string:x-canonical-private-synchronous:kdefusion",
            &title,
            &body,
        ])
        .status()
        .await;
}

pub async fn error(title: impl AsRef<str>, body: impl AsRef<str>) {
    send_urgency(title, body, "critical").await;
}
