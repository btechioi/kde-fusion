//! In-app installation of the streaming packages.
//!
//! The package manager and package names differ between distributions.  This
//! module resolves them through [`crate::platform`], installs everything that
//! is available in the configured repositories through `pkexec` (Plasma's
//! Polkit agent asks for the administrator password), and exposes the
//! vendor-specific commands for packages that are only distributed outside the
//! distribution repositories (Sunshine on Fedora/Debian, Moonlight on
//! Fedora/Debian).
//!
//! The GUI polls the install status through the loopback API while the
//! transaction runs.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::moonlight;
use crate::platform::{self, Distro, Family};
use crate::util::{command_exists, now_unix};

const LOG_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct PackageState {
    /// Executable used to detect the package.
    pub binary: &'static str,
    /// Distribution package name, or the vendor label.
    pub package: String,
    pub label: &'static str,
    pub purpose: &'static str,
    pub installed: bool,
    pub required: bool,
    /// repository | copr | flatpak | manual
    pub method: String,
    /// Explanation shown in the GUI for vendor packages.
    pub note: String,
    /// Copy/paste command for manual installs (and the automated command for
    /// vendor packages).
    pub command: String,
    /// Download/documentation page for manual installs.
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct InstallStatus {
    pub running: bool,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub manager: String,
    pub command: String,
    pub started_at: u64,
    pub finished_at: u64,
    pub log: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallOverview {
    /// Human readable distribution name, e.g. "Fedora Linux 44 (dnf)".
    pub distro: String,
    /// arch | fedora | debian | suse | other
    pub family: String,
    pub manager: String,
    pub helper: String,
    pub can_install: bool,
    /// Why automatic installation is unavailable, if it is.
    pub reason: String,
    /// All automated commands, joined for display.
    pub command: String,
    pub packages: Vec<PackageState>,
    /// Package names that are missing.
    pub missing: Vec<String>,
    /// Labels of missing packages that require a manual vendor step.
    pub manual: Vec<String>,
    pub status: InstallStatus,
}

#[derive(Debug, Clone)]
struct Step {
    argv: Vec<String>,
    elevated: bool,
    display: String,
}

#[derive(Debug, Clone)]
struct InstallPlan {
    manager: String,
    helper: String,
    steps: Vec<Step>,
    display: String,
    can_install: bool,
    reason: String,
}

pub struct Installer {
    status: Mutex<InstallStatus>,
}

impl Default for Installer {
    fn default() -> Self {
        Self::new()
    }
}

impl Installer {
    pub fn new() -> Self {
        Self {
            status: Mutex::new(InstallStatus::default()),
        }
    }

    pub async fn status(&self) -> InstallStatus {
        self.status.lock().await.clone()
    }

    pub async fn overview(&self) -> InstallOverview {
        let distro = platform::detect();
        let packages = packages_for(&distro);
        let plan = plan_for(&distro, &packages);
        let missing = packages
            .iter()
            .filter(|package| package.required && !package.installed)
            .map(|package| package.package.clone())
            .collect::<Vec<_>>();
        let manual = packages
            .iter()
            .filter(|package| package.required && !package.installed && package.method == "manual")
            .map(|package| package.label.to_string())
            .collect::<Vec<_>>();
        InstallOverview {
            distro: distro.summary(),
            family: platform::family_for(&distro).id().to_string(),
            manager: plan.manager.clone(),
            helper: plan.helper.clone(),
            can_install: plan.can_install,
            reason: plan.reason.clone(),
            command: plan.display.clone(),
            packages,
            missing,
            manual,
            status: self.status().await,
        }
    }

    /// Start an asynchronous installation.  Returns as soon as the first
    /// package-manager process has been spawned; progress is available through
    /// [`Self::status`].
    pub async fn start(self: &Arc<Self>) -> Result<()> {
        let distro = platform::detect();
        let packages = packages_for(&distro);
        let plan = plan_for(&distro, &packages);
        if !plan.can_install {
            bail!("{}", plan.reason);
        }
        if plan.steps.is_empty() {
            bail!("there is nothing to install automatically");
        }

        {
            let mut status = self.status.lock().await;
            if status.running {
                bail!("a package installation is already running");
            }
            *status = InstallStatus {
                running: true,
                manager: plan.manager.clone(),
                command: plan.display.clone(),
                started_at: now_unix(),
                ..Default::default()
            };
        }

        let installer = Arc::clone(self);
        tokio::spawn(async move {
            let mut outcome: Result<Option<i32>> = Ok(Some(0));
            for step in &plan.steps {
                installer.append_log(format!("$ {}", step.display)).await;
                let mut command = build_command(step, &plan.helper);
                command
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .env("LC_ALL", "C")
                    .env("DEBIAN_FRONTEND", "noninteractive");
                let child = match command.spawn() {
                    Ok(child) => child,
                    Err(err) => {
                        outcome = Err(err).with_context(|| format!("run {}", step.display));
                        break;
                    }
                };
                match supervise(installer.clone(), child).await {
                    Ok(Some(0)) => {}
                    Ok(code) => {
                        outcome = Ok(code);
                        break;
                    }
                    Err(err) => {
                        outcome = Err(err);
                        break;
                    }
                }
            }

            let mut status = installer.status.lock().await;
            status.running = false;
            status.finished_at = now_unix();
            match outcome {
                Ok(Some(0)) => {
                    status.exit_code = Some(0);
                    status.success = true;
                }
                Ok(code) => {
                    status.exit_code = code;
                    status.error = format!(
                        "The package manager exited with {}. See the log for details.",
                        code.map(|value| value.to_string())
                            .unwrap_or_else(|| "a signal".into())
                    );
                }
                Err(err) => {
                    status.error = format!("{err:#}");
                }
            }
        });

        Ok(())
    }

    async fn append_log(&self, line: String) {
        let mut status = self.status.lock().await;
        if status.log.len() >= LOG_LIMIT {
            return;
        }
        status.log.push_str(&line);
        status.log.push('\n');
    }
}

fn build_command(step: &Step, helper: &str) -> Command {
    if step.elevated && helper == "pkexec" {
        let mut command = Command::new("pkexec");
        command.args(&step.argv);
        return command;
    }
    // Root runs directly; user-level steps (Flatpak) also run directly.
    let mut command = Command::new(&step.argv[0]);
    command.args(&step.argv[1..]);
    command
}

async fn supervise(installer: Arc<Installer>, mut child: Child) -> Result<Option<i32>> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let mut readers = Vec::new();
    if let Some(stdout) = stdout {
        readers.push(tokio::spawn(read_stream(installer.clone(), stdout)));
    }
    if let Some(stderr) = stderr {
        readers.push(tokio::spawn(read_stream(installer.clone(), stderr)));
    }
    let status = child.wait().await.context("wait for the package manager")?;
    for reader in readers {
        let _ = reader.await;
    }
    Ok(status.code())
}

async fn read_stream(installer: Arc<Installer>, stream: impl AsyncRead + Unpin) {
    let mut lines = BufReader::new(stream).lines();
    loop {
        match tokio::time::timeout(Duration::from_millis(250), lines.next_line()).await {
            Ok(Ok(Some(line))) => installer.append_log(line).await,
            Ok(Ok(None)) => break,
            Ok(Err(_)) => break,
            Err(_) => {
                // Keep waiting: package downloads can be silent for a while.
                if installer.status.lock().await.running {
                    continue;
                }
                break;
            }
        }
    }
}

fn packages_for(distro: &Distro) -> Vec<PackageState> {
    let family = platform::family_for(distro);
    vec![
        sunshine_state(family),
        moonlight_state(family),
        krfb_state(family),
    ]
}

fn base_state(
    binary: &'static str,
    label: &'static str,
    purpose: &'static str,
    installed: bool,
) -> PackageState {
    PackageState {
        binary,
        package: binary.to_string(),
        label,
        purpose,
        installed,
        required: true,
        method: "repository".into(),
        note: String::new(),
        command: String::new(),
        url: String::new(),
    }
}

fn repository_state(
    mut state: PackageState,
    family: Family,
    package: &'static str,
) -> PackageState {
    state.package = package.to_string();
    state.method = "repository".into();
    state.command = repository_command(family, package);
    state
}

fn repository_command(family: Family, package: &str) -> String {
    match family {
        Family::Arch => format!("sudo pacman -S {package}"),
        Family::Fedora => format!("sudo dnf install {package}"),
        Family::Debian => format!("sudo apt install {package}"),
        Family::Suse => format!("sudo zypper install {package}"),
        Family::Other => format!("Install {package} with your package manager"),
    }
}

fn vendor_state(
    mut state: PackageState,
    family: Family,
    key: &str,
    default_package: &str,
) -> PackageState {
    match platform::vendor_guidance(key, family) {
        Some(guidance) => {
            state.method = if guidance.command.starts_with("flatpak ") {
                "flatpak".into()
            } else if guidance.command.contains("copr") {
                "copr".into()
            } else {
                "manual".into()
            };
            state.package = default_package.to_string();
            state.note = guidance.detail.to_string();
            state.command = guidance.command.to_string();
            state.url = guidance.url.to_string();
        }
        None => {
            state.package = default_package.to_string();
            state.method = "manual".into();
            state.note = "Install this package with the official upstream instructions.".into();
            state.command = format!("Install {default_package} for your distribution.");
        }
    }
    state
}

fn sunshine_state(family: Family) -> PackageState {
    let state = base_state(
        "sunshine",
        "Sunshine",
        "Hosts the stream and captures the KDE virtual output.",
        command_exists("sunshine"),
    );
    match family {
        Family::Arch => repository_state(state, family, "sunshine"),
        _ => vendor_state(state, family, "sunshine", "sunshine"),
    }
}

fn moonlight_state(family: Family) -> PackageState {
    let install = moonlight::installation();
    let mut state = base_state(
        "moonlight",
        "Moonlight",
        "Connects to the other PC and displays the stream.",
        install.available,
    );
    if install.kind == "flatpak" {
        state.package = "Flatpak".into();
        state.method = "flatpak".into();
        state.command = format!("flatpak run {}", moonlight::FLATPAK_APP);
        state.note = install.detail;
        return state;
    }
    match family {
        Family::Arch => repository_state(state, family, "moonlight-qt"),
        _ => {
            let has_flatpak = command_exists("flatpak");
            let mut state = vendor_state(state, family, "moonlight", "moonlight-qt");
            if state.method == "flatpak" && !has_flatpak {
                state.method = "manual".into();
                state.command = "Install Flatpak, then install com.moonlight_stream.Moonlight \
                                 from Flathub."
                    .into();
            }
            state
        }
    }
}

fn krfb_state(family: Family) -> PackageState {
    let state = base_state(
        "krfb-virtualmonitor",
        "KDE virtual monitor",
        "Creates the Virtual-KDEFusion Wayland output.",
        command_exists("krfb-virtualmonitor"),
    );
    repository_state(state, family, "krfb")
}

fn plan_for(distro: &Distro, packages: &[PackageState]) -> InstallPlan {
    let manager = distro.manager.clone();
    let missing: Vec<&PackageState> = packages
        .iter()
        .filter(|package| package.required && !package.installed)
        .collect();

    if missing.is_empty() {
        return InstallPlan {
            manager,
            helper: String::new(),
            steps: Vec::new(),
            display: String::new(),
            can_install: false,
            reason: "Sunshine, Moonlight and the KDE virtual monitor are already installed."
                .to_string(),
        };
    }

    let mut steps = Vec::new();

    let repository_packages: Vec<String> = missing
        .iter()
        .filter(|package| package.method == "repository")
        .map(|package| package.package.clone())
        .collect();
    if !repository_packages.is_empty() {
        steps.extend(install_steps(&manager, &repository_packages));
    }

    if missing.iter().any(|package| package.method == "copr") && manager == "dnf" {
        steps.push(step(
            &["dnf", "copr", "enable", "-y", "lizardbyte/stable"],
            true,
        ));
        steps.push(step(&["dnf", "install", "-y", "Sunshine"], true));
    }

    if missing.iter().any(|package| package.method == "flatpak") {
        steps.push(step(
            &[
                "flatpak",
                "remote-add",
                "--if-not-exists",
                "flathub",
                "https://flathub.org/repo/flathub.flatpakrepo",
            ],
            false,
        ));
        steps.push(step(
            &[
                "flatpak",
                "install",
                "-y",
                "flathub",
                moonlight::FLATPAK_APP,
            ],
            false,
        ));
    }

    let manual: Vec<&PackageState> = missing
        .iter()
        .filter(|package| package.method == "manual")
        .copied()
        .collect();

    let needs_elevation = steps.iter().any(|step| step.elevated);
    let can_install = if steps.is_empty() {
        false
    } else if needs_elevation {
        platform::is_root() || command_exists("pkexec")
    } else {
        true
    };

    let helper = if !needs_elevation {
        "user".to_string()
    } else if platform::is_root() {
        "root".to_string()
    } else if command_exists("pkexec") {
        "pkexec".to_string()
    } else {
        distro.elevation.clone()
    };

    let display = steps
        .iter()
        .map(|step| step.display.clone())
        .collect::<Vec<_>>()
        .join(" && ");

    let reason = if !can_install {
        if steps.is_empty() {
            if manual.is_empty() {
                format!(
                    "No supported package manager was found. Update your system manually and run \
                     Diagnostics again."
                )
            } else {
                let labels = manual
                    .iter()
                    .map(|package| package.label)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "Automatic installation is not available for {labels}. Use the commands \
                     below, then run Diagnostics again."
                )
            }
        } else {
            "pkexec (Polkit) is not available. Run the command shown below in a terminal."
                .to_string()
        }
    } else if manual.is_empty() {
        String::new()
    } else {
        let labels = manual
            .iter()
            .map(|package| package.label)
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "The available packages will be installed automatically. {labels} need the manual \
             step shown below."
        )
    };

    InstallPlan {
        manager,
        helper,
        steps,
        display,
        can_install,
        reason,
    }
}

fn install_steps(manager: &str, packages: &[String]) -> Vec<Step> {
    match manager {
        "pacman" => {
            let mut argv = vec![
                "pacman".to_string(),
                "-S".into(),
                "--needed".into(),
                "--noconfirm".into(),
            ];
            argv.extend(packages.iter().cloned());
            vec![Step {
                display: display_argv(&argv),
                argv,
                elevated: true,
            }]
        }
        "dnf" => {
            let mut argv = vec!["dnf".to_string(), "install".into(), "-y".into()];
            argv.extend(packages.iter().cloned());
            vec![Step {
                display: display_argv(&argv),
                argv,
                elevated: true,
            }]
        }
        "apt-get" | "apt" => {
            let mut install = vec!["apt-get".to_string(), "install".into(), "-y".into()];
            install.extend(packages.iter().cloned());
            vec![
                step(&["apt-get", "update"], true),
                Step {
                    display: display_argv(&install),
                    argv: install,
                    elevated: true,
                },
            ]
        }
        "zypper" => {
            let mut argv = vec![
                "zypper".to_string(),
                "--non-interactive".into(),
                "install".into(),
            ];
            argv.extend(packages.iter().cloned());
            vec![Step {
                display: display_argv(&argv),
                argv,
                elevated: true,
            }]
        }
        _ => Vec::new(),
    }
}

fn step(argv: &[&str], elevated: bool) -> Step {
    let argv = argv
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    Step {
        display: display_argv(&argv),
        argv,
        elevated,
    }
}

fn display_argv(argv: &[String]) -> String {
    argv.iter()
        .map(|value| {
            if value
                .chars()
                .any(|c| c.is_whitespace() || c == '"' || c == '\'')
            {
                format!("{value:?}")
            } else {
                value.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distro(family: Family, manager: &str) -> Distro {
        Distro {
            id: "test".into(),
            name: "Test Linux".into(),
            version: "1".into(),
            codename: String::new(),
            family,
            family_label: family.label().into(),
            manager: manager.into(),
            elevation: "pkexec".into(),
        }
    }

    fn missing_package(label: &'static str, package: &str, method: &str) -> PackageState {
        PackageState {
            binary: "test",
            package: package.into(),
            label,
            purpose: "test",
            installed: false,
            required: true,
            method: method.into(),
            note: String::new(),
            command: String::new(),
            url: String::new(),
        }
    }

    #[test]
    fn detects_required_packages() {
        let packages = packages_for(&platform::detect());
        assert!(packages.iter().any(|p| p.binary == "sunshine"));
        assert!(packages.iter().any(|p| p.binary == "moonlight"));
        assert!(packages.iter().any(|p| p.binary == "krfb-virtualmonitor"));
    }

    #[test]
    fn arch_uses_native_packages() {
        let distro = distro(Family::Arch, "pacman");
        let packages = vec![
            missing_package("Sunshine", "sunshine", "repository"),
            missing_package("Moonlight", "moonlight-qt", "repository"),
            missing_package("KDE virtual monitor", "krfb", "repository"),
        ];
        let plan = plan_for(&distro, &packages);
        assert!(plan.display.contains("pacman -S"));
        assert!(plan.display.contains("sunshine"));
        assert!(plan.display.contains("moonlight-qt"));
    }

    #[test]
    fn fedora_uses_copr_and_flatpak() {
        let distro = distro(Family::Fedora, "dnf");
        let packages = vec![
            missing_package("Sunshine", "Sunshine", "copr"),
            missing_package("Moonlight", "moonlight-qt", "flatpak"),
            missing_package("KDE virtual monitor", "krfb", "repository"),
        ];
        let plan = plan_for(&distro, &packages);
        assert!(plan.display.contains("dnf copr enable"));
        assert!(plan.display.contains("flatpak install"));
        assert!(plan.display.contains("krfb"));
    }

    #[test]
    fn debian_updates_package_lists() {
        let distro = distro(Family::Debian, "apt-get");
        let packages = vec![missing_package("KDE virtual monitor", "krfb", "repository")];
        let plan = plan_for(&distro, &packages);
        assert!(plan.display.contains("apt-get update"));
        assert!(plan.display.contains("apt-get install -y krfb"));
    }

    #[test]
    fn manual_only_plan_is_not_runnable() {
        let distro = distro(Family::Debian, "apt-get");
        let packages = vec![missing_package("Sunshine", "sunshine", "manual")];
        let plan = plan_for(&distro, &packages);
        assert!(!plan.can_install);
        assert!(plan.steps.is_empty());
        assert!(plan.reason.contains("Sunshine"));
    }

    #[test]
    fn complete_plan_has_no_steps() {
        let distro = distro(Family::Arch, "pacman");
        let packages = vec![PackageState {
            installed: true,
            ..missing_package("Sunshine", "sunshine", "repository")
        }];
        let plan = plan_for(&distro, &packages);
        assert!(!plan.can_install);
        assert!(plan.reason.contains("already installed"));
    }
}
