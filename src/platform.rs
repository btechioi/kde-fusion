//! Distribution detection and cross-distro package naming.
//!
//! KDE Fusion itself is a plain Rust binary and runs anywhere, but the
//! packages around it are named differently on Arch, Fedora and Debian.
//! Everything that shells out to a package manager goes through this module so
//! the GUI, diagnostics and installer agree on the right names and commands.

use serde::Serialize;
use std::collections::HashMap;

use crate::util::command_exists;

/// Package-family buckets used for package naming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    Arch,
    Fedora,
    Debian,
    Suse,
    Other,
}

impl Family {
    pub fn id(self) -> &'static str {
        match self {
            Family::Arch => "arch",
            Family::Fedora => "fedora",
            Family::Debian => "debian",
            Family::Suse => "suse",
            Family::Other => "other",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Family::Arch => "Arch Linux family",
            Family::Fedora => "Fedora family",
            Family::Debian => "Debian/Ubuntu family",
            Family::Suse => "openSUSE family",
            Family::Other => "Linux",
        }
    }

    /// The package manager used by the family, if it is installed.
    pub fn package_manager(self) -> &'static str {
        match self {
            Family::Arch => "pacman",
            Family::Fedora => "dnf",
            Family::Debian => "apt-get",
            Family::Suse => "zypper",
            Family::Other => "",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Distro {
    /// `ID` from os-release, e.g. `fedora`, `debian`, `cachyos`.
    pub id: String,
    /// Human readable name, e.g. `Fedora Linux 44`.
    pub name: String,
    /// `VERSION_ID` when present.
    pub version: String,
    /// `VERSION_CODENAME` when present (Debian/Ubuntu).
    pub codename: String,
    pub family: Family,
    pub family_label: String,
    /// Detected package manager binary, or an empty string.
    pub manager: String,
    /// `root`, `pkexec`, `sudo` or an empty string.
    pub elevation: String,
}

impl Distro {
    pub fn summary(&self) -> String {
        let mut text = self.name.clone();
        if !self.version.is_empty() && !self.name.contains(&self.version) {
            text.push(' ');
            text.push_str(&self.version);
        }
        if text.is_empty() {
            text = self.family_label.to_string();
        }
        match self.manager.as_str() {
            "" => text,
            manager => format!("{text} ({manager})"),
        }
    }
}

pub fn detect() -> Distro {
    let fields = read_os_release();
    let id = fields.get("ID").cloned().unwrap_or_default();
    let id_like = fields.get("ID_LIKE").cloned().unwrap_or_default();
    let name = fields
        .get("PRETTY_NAME")
        .or_else(|| fields.get("NAME"))
        .cloned()
        .unwrap_or_else(|| id.clone());
    let version = fields.get("VERSION_ID").cloned().unwrap_or_default();
    let codename = fields.get("VERSION_CODENAME").cloned().unwrap_or_default();
    let family = classify(&id, &id_like);
    let manager = detect_manager(family);
    Distro {
        id,
        name,
        version,
        codename,
        family,
        family_label: family.label().to_string(),
        manager,
        elevation: detect_elevation(),
    }
}

fn read_os_release() -> HashMap<String, String> {
    for path in ["/etc/os-release", "/usr/lib/os-release"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            let parsed = parse_os_release(&text);
            if !parsed.is_empty() {
                return parsed;
            }
        }
    }
    HashMap::new()
}

/// Minimal os-release parser.  Handles `KEY=value`, `KEY="value"` and the
/// escaped quotes used by some distributions.
pub fn parse_os_release(text: &str) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let mut value = raw.trim().to_string();
        if value.len() >= 2 {
            let first = value.as_bytes()[0];
            let last = value.as_bytes()[value.len() - 1];
            if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
                value = value[1..value.len() - 1].to_string();
            }
        }
        value = value
            .replace("\\\"", "\"")
            .replace("\\'", "'")
            .replace("\\\\", "\\");
        fields.insert(key.to_string(), value);
    }
    fields
}

pub fn classify(id: &str, id_like: &str) -> Family {
    let mut ids: Vec<String> = vec![id.to_lowercase()];
    ids.extend(id_like.split_whitespace().map(|value| value.to_lowercase()));
    let has = |needle: &str| ids.iter().any(|value| value == needle);

    if has("arch")
        || has("cachyos")
        || has("manjaro")
        || has("endeavouros")
        || has("endeavour")
        || has("garuda")
        || has("artix")
    {
        return Family::Arch;
    }
    if has("fedora")
        || has("rhel")
        || has("centos")
        || has("rocky")
        || has("almalinux")
        || has("nobara")
    {
        return Family::Fedora;
    }
    if has("debian")
        || has("ubuntu")
        || has("linuxmint")
        || has("pop")
        || has("elementary")
        || has("zorin")
        || has("kali")
        || has("neon")
        || has("tuxedo")
    {
        return Family::Debian;
    }
    if ids.iter().any(|value| value.contains("suse")) {
        return Family::Suse;
    }
    Family::Other
}

fn detect_manager(family: Family) -> String {
    let preferred = family.package_manager();
    if !preferred.is_empty() && command_exists(preferred) {
        return preferred.to_string();
    }
    for candidate in ["pacman", "dnf", "apt-get", "zypper"] {
        if command_exists(candidate) {
            return candidate.to_string();
        }
    }
    if command_exists("rpm-ostree") {
        return "rpm-ostree".to_string();
    }
    String::new()
}

pub fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("Uid:"))
                .and_then(|line| line.split_whitespace().nth(1).map(|uid| uid == "0"))
        })
        .unwrap_or(false)
}

fn detect_elevation() -> String {
    if is_root() {
        return "root".into();
    }
    if command_exists("pkexec") {
        return "pkexec".into();
    }
    if command_exists("sudo") {
        return "sudo".into();
    }
    String::new()
}

/// Map a command or package key to the distribution package that provides it.
pub fn package_name(key: &str, family: Family) -> Option<&'static str> {
    let value = match key {
        "kscreen-doctor" | "kscreen" => "kscreen",
        "krfb-virtualmonitor" | "krfb" => "krfb",
        "curl" => "curl",
        "notify-send" => match family {
            Family::Debian => "libnotify-bin",
            Family::Suse => "libnotify-tools",
            _ => "libnotify",
        },
        "secret-tool" => match family {
            Family::Debian | Family::Suse => "libsecret-tools",
            _ => "libsecret",
        },
        "vainfo" => match family {
            Family::Debian => "vainfo",
            _ => "libva-utils",
        },
        "sunshine" | "moonlight" => return None,
        _ => return None,
    };
    Some(value)
}

pub struct VendorGuidance {
    pub title: &'static str,
    pub detail: &'static str,
    pub command: &'static str,
    pub url: &'static str,
}

const SUNSHINE_URL: &str = "https://github.com/LizardByte/Sunshine/releases/latest";
const SUNSHINE_DOCS: &str = "https://docs.lizardbyte.dev/projects/sunshine/latest/";
const MOONLIGHT_URL: &str = "https://moonlight-stream.org/";

/// Installation guidance for packages that are not in the distribution
/// repositories (Sunshine and Moonlight on Fedora/Debian).
pub fn vendor_guidance(key: &str, family: Family) -> Option<VendorGuidance> {
    match (key, family) {
        ("sunshine", Family::Arch) => None,
        ("sunshine", Family::Fedora) => Some(VendorGuidance {
            title: "Sunshine (LizardByte COPR)",
            detail: "Sunshine is not in the Fedora repositories. Enable LizardByte's official \
                     COPR, then install the case-sensitive package `Sunshine`.",
            command: "sudo dnf copr enable lizardbyte/stable && sudo dnf install Sunshine",
            url: SUNSHINE_DOCS,
        }),
        ("sunshine", Family::Debian) => Some(VendorGuidance {
            title: "Sunshine (official Debian package)",
            detail: "Sunshine is not in the Debian repositories. Download the .deb for your \
                     release (Debian trixie/bookworm or Ubuntu) from the official releases page \
                     and install it with apt.",
            command: "sudo apt install ./sunshine-*.deb",
            url: SUNSHINE_URL,
        }),
        ("sunshine", Family::Suse) => Some(VendorGuidance {
            title: "Sunshine (LizardByte RPM)",
            detail: "Download the openSUSE RPM from the official releases page and install it \
                     with zypper.",
            command: "sudo zypper install ./Sunshine-*.rpm",
            url: SUNSHINE_URL,
        }),
        ("sunshine", Family::Other) => Some(VendorGuidance {
            title: "Sunshine",
            detail: "Install Sunshine from the official releases page for your distribution.",
            command: "Install Sunshine, then run Diagnostics again.",
            url: SUNSHINE_URL,
        }),

        ("moonlight", Family::Arch) => None,
        ("moonlight", Family::Fedora) => Some(VendorGuidance {
            title: "Moonlight (Flatpak)",
            detail: "Moonlight is not in the Fedora repositories. The Flatpak build works with \
                     KDE Fusion, or install the AppImage as `moonlight` in ~/.local/bin.",
            command: "flatpak install -y flathub com.moonlight_stream.Moonlight",
            url: MOONLIGHT_URL,
        }),
        ("moonlight", Family::Debian) => Some(VendorGuidance {
            title: "Moonlight (Flatpak or Snap)",
            detail: "Moonlight is not in the Debian repositories. The Flatpak build works with \
                     KDE Fusion, or use the AppImage from the Moonlight website.",
            command: "flatpak install -y flathub com.moonlight_stream.Moonlight",
            url: MOONLIGHT_URL,
        }),
        ("moonlight", Family::Suse) => Some(VendorGuidance {
            title: "Moonlight (Flatpak or AppImage)",
            detail: "Install Moonlight from Flathub or use the upstream AppImage.",
            command: "flatpak install -y flathub com.moonlight_stream.Moonlight",
            url: MOONLIGHT_URL,
        }),
        ("moonlight", Family::Other) => Some(VendorGuidance {
            title: "Moonlight",
            detail: "Install Moonlight from the official website or your distribution.",
            command: "Install Moonlight, then run Diagnostics again.",
            url: MOONLIGHT_URL,
        }),
        _ => None,
    }
}

fn install_verb(manager: &str) -> Option<&'static str> {
    match manager {
        "pacman" => Some("sudo pacman -S"),
        "dnf" => Some("sudo dnf install"),
        "apt-get" | "apt" => Some("sudo apt install"),
        "zypper" => Some("sudo zypper install"),
        _ => None,
    }
}

/// A copy/pasteable command that installs the package providing `key`.
pub fn install_hint(key: &str) -> String {
    let distro = detect();
    if let Some(guidance) = vendor_guidance(key, family_for(&distro)) {
        return format!("{}: {}", guidance.title, guidance.command);
    }
    if let Some(package) = package_name(key, family_for(&distro)) {
        if let Some(verb) = install_verb(&distro.manager) {
            return format!("{verb} {package}");
        }
        return format!("Install the '{package}' package with your distribution package manager.");
    }
    if let Some(guidance) = vendor_guidance(key, Family::Other) {
        return format!("{}: {}", guidance.title, guidance.command);
    }
    format!("Install '{key}' with your distribution package manager.")
}

/// Encoder driver hint for the current distribution.
pub fn encoder_hint() -> String {
    let distro = detect();
    match family_for(&distro) {
        Family::Debian => {
            "sudo apt install intel-media-va-driver mesa-va-drivers vainfo".to_string()
        }
        Family::Fedora => {
            "sudo dnf install intel-media-driver libva-intel-driver libva-utils (Intel packages \
             come from RPM Fusion)"
                .to_string()
        }
        Family::Arch => "sudo pacman -S intel-media-driver libva-utils".to_string(),
        Family::Suse => "sudo zypper install intel-media-driver libva-utils".to_string(),
        Family::Other => {
            "Install a VA-API driver for your GPU and verify it with vainfo.".to_string()
        }
    }
}

/// PipeWire Pulse compatibility package hint.
pub fn audio_hint() -> String {
    let distro = detect();
    let package = match family_for(&distro) {
        Family::Fedora | Family::Suse => "pipewire-pulseaudio",
        _ => "pipewire-pulse",
    };
    match install_verb(&distro.manager) {
        Some(verb) => format!("{verb} {package}"),
        None => format!("Install {package} with your distribution package manager."),
    }
}

/// QML runtime package hint for launcher error messages.
pub fn qml_runtime_hint() -> &'static str {
    "Install the Qt 6 QML runtime (Arch: qt6-declarative, Fedora: qt6-qtdeclarative-devel, \
     Debian/Ubuntu: qml-qt6) or set KDEFUSION_QML_RUNTIME."
}

/// If the distro reports `Other` but a known package manager is installed,
/// treat it like the family that manager represents.
pub fn family_for(distro: &Distro) -> Family {
    match distro.manager.as_str() {
        "pacman" => Family::Arch,
        "dnf" => Family::Fedora,
        "apt-get" | "apt" => Family::Debian,
        "zypper" => Family::Suse,
        _ => distro.family,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_and_plain_values() {
        let fields = parse_os_release(
            "ID=debian\nPRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\nVERSION_CODENAME=trixie\n# comment\n",
        );
        assert_eq!(fields.get("ID").map(String::as_str), Some("debian"));
        assert_eq!(
            fields.get("PRETTY_NAME").map(String::as_str),
            Some("Debian GNU/Linux 13 (trixie)")
        );
        assert_eq!(
            fields.get("VERSION_CODENAME").map(String::as_str),
            Some("trixie")
        );
    }

    #[test]
    fn classifies_families() {
        assert_eq!(classify("cachyos", "arch"), Family::Arch);
        assert_eq!(classify("manjaro", ""), Family::Arch);
        assert_eq!(classify("fedora", ""), Family::Fedora);
        assert_eq!(classify("nobara", "fedora"), Family::Fedora);
        assert_eq!(classify("debian", ""), Family::Debian);
        assert_eq!(classify("ubuntu", "debian"), Family::Debian);
        assert_eq!(
            classify("opensuse-tumbleweed", "suse opensuse"),
            Family::Suse
        );
        assert_eq!(classify("gentoo", ""), Family::Other);
    }

    #[test]
    fn package_names_follow_distribution_conventions() {
        assert_eq!(
            package_name("kscreen-doctor", Family::Arch),
            Some("kscreen")
        );
        assert_eq!(
            package_name("notify-send", Family::Debian),
            Some("libnotify-bin")
        );
        assert_eq!(
            package_name("notify-send", Family::Fedora),
            Some("libnotify")
        );
        assert_eq!(
            package_name("secret-tool", Family::Debian),
            Some("libsecret-tools")
        );
        assert_eq!(package_name("vainfo", Family::Debian), Some("vainfo"));
        assert_eq!(package_name("vainfo", Family::Arch), Some("libva-utils"));
        assert_eq!(package_name("sunshine", Family::Arch), None);
    }

    #[test]
    fn vendor_guidance_is_family_specific() {
        assert!(vendor_guidance("sunshine", Family::Arch).is_none());
        let fedora = vendor_guidance("sunshine", Family::Fedora).expect("fedora guidance");
        assert!(fedora.command.contains("copr"));
        let debian = vendor_guidance("sunshine", Family::Debian).expect("debian guidance");
        assert!(debian.command.contains("apt"));
        assert!(vendor_guidance("moonlight", Family::Debian).is_some());
    }

    #[test]
    fn manager_maps_back_to_family() {
        let distro = Distro {
            id: "custom".into(),
            name: "Custom".into(),
            version: String::new(),
            codename: String::new(),
            family: Family::Other,
            family_label: Family::Other.label().into(),
            manager: "apt-get".into(),
            elevation: "pkexec".into(),
        };
        assert_eq!(family_for(&distro), Family::Debian);
    }
}
