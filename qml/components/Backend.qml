import QtQuick

/// Small HTTP client for the loopback KDE Fusion API.
///
/// The daemon binds to 127.0.0.1 only, so no authentication is needed here.
/// Every call sets `lastError` on failure and the UI shows it as a passive
/// notification.
QtObject {
    id: backend

    /// Change this if the daemon API port was changed in config.toml.
    property string apiBase: "http://127.0.0.1:47812"

    property bool connected: false
    property string lastError: ""
    property var status: ({})
    property var peers: []
    property var pending: []
    property var config: ({})
    property var diagnostics: ({})
    property bool diagnosticsLoading: false
    property var install: ({})
    property bool installRunning: false
    property bool installPolling: false
    property string pairingCode: ""
    property string pairingPeerName: ""
    property bool pairingDialogVisible: false

    /// Emitted when the tray asks this window to come to the front.
    signal openRequested(string page)
    property int lastOpenRequest: -1

    readonly property var session: status && status.session ? status.session : ({})
    readonly property var pairedPeers: (peers || []).filter(function (peer) {
        return peer.paired === true;
    })
    readonly property var nearbyPeers: (peers || []).filter(function (peer) {
        return peer.paired !== true;
    })
    readonly property string deviceName: status && status.device ? status.device.name : ""
    readonly property string deviceId: status && status.device ? status.device.id : ""
    readonly property bool busy: session.phase !== undefined && session.phase !== "idle" && session.phase !== "failed"
    readonly property bool active: session.phase === "streaming"
    readonly property var installPackages: install && install.packages ? install.packages : []
    readonly property var installMissing: install && install.missing ? install.missing : []
    readonly property var installManual: install && install.manual ? install.manual : []

    function call(method, path, body, onSuccess, onError) {
        var xhr = new XMLHttpRequest();
        xhr.open(method, apiBase + path);
        xhr.setRequestHeader("Content-Type", "application/json");
        xhr.timeout = method === "POST" && path === "/api/session/start" ? 180000 : 15000;
        xhr.onreadystatechange = function () {
            if (xhr.readyState !== XMLHttpRequest.DONE) {
                return;
            }
            var payload = null;
            try {
                payload = xhr.responseText ? JSON.parse(xhr.responseText) : null;
            } catch (error) {
                payload = null;
            }
            if (xhr.status >= 200 && xhr.status < 300 && payload && payload.ok !== false) {
                backend.connected = true;
                backend.lastError = "";
                if (onSuccess) {
                    onSuccess(payload);
                }
                return;
            }
            var message = payload && payload.error
                ? payload.error
                : (xhr.status === 0
                    ? "The KDE Fusion daemon is not running (start it with: systemctl --user start kdefusiond)"
                    : "Request failed with HTTP " + xhr.status);
            backend.connected = xhr.status !== 0;
            backend.lastError = message;
            if (onError) {
                onError(message);
            }
        };
        xhr.onerror = function () {
            backend.connected = false;
            backend.lastError = "Cannot reach the KDE Fusion daemon at " + apiBase;
            if (onError) {
                onError(backend.lastError);
            }
        };
        xhr.send(body ? JSON.stringify(body) : null);
    }

    function refreshStatus() {
        call("GET", "/api/status", null, function (data) {
            status = data;
            connected = true;
            var ui = data.ui || {};
            var request = ui.open_request || 0;
            if (lastOpenRequest >= 0 && request > lastOpenRequest) {
                openRequested(ui.page || "");
            }
            lastOpenRequest = request;
        });
    }

    function refreshPeers() {
        call("GET", "/api/peers", null, function (data) {
            peers = data.peers || [];
        });
    }

    function refreshPending() {
        call("GET", "/api/pairing/pending", null, function (data) {
            pending = data.pending || [];
        });
    }

    function refreshConfig() {
        call("GET", "/api/config", null, function (data) {
            config = data.config || {};
        });
    }

    function refreshAll() {
        refreshStatus();
        refreshPeers();
        refreshPending();
        refreshConfig();
        refreshInstall();
    }

    function pair(peerId, code) {
        var body = { peer_id: peerId };
        if (code) {
            body.code = code;
        }
        call("POST", "/api/peers/pair", body, function (data) {
            lastError = "";
            pairingCode = data.code || "";
            pairingPeerName = data.peer_name || peerId;
            pairingDialogVisible = true;
            refreshPeers();
            refreshPending();
        });
    }

    function approve(requestId) {
        call("POST", "/api/pairing/approve", { request_id: requestId }, function () {
            refreshPeers();
            refreshPending();
        });
    }

    function reject(requestId) {
        call("POST", "/api/pairing/reject", { request_id: requestId }, function () {
            refreshPending();
        });
    }

    function removePeer(peerId) {
        call("POST", "/api/peers/remove", { peer_id: peerId }, function () {
            refreshPeers();
        });
    }

    function startSession(request, onSuccess) {
        call("POST", "/api/session/start", request, function (data) {
            refreshStatus();
            if (onSuccess) {
                onSuccess(data);
            }
        });
    }

    function stopSession(reason) {
        call("POST", "/api/session/stop", { reason: reason || "user requested" }, function () {
            refreshStatus();
        });
    }

    function saveConfig(patch, onSuccess) {
        call("POST", "/api/config", patch, function (data) {
            config = data.config || config;
            refreshConfig();
            if (onSuccess) {
                onSuccess(data);
            }
        });
    }

    function saveSunshineCredentials(username, password, onSuccess) {
        call("POST", "/api/sunshine/credentials", {
            username: username,
            password: password
        }, function (data) {
            if (onSuccess) {
                onSuccess(data);
            }
        });
    }

    function prepareSunshine(onSuccess) {
        call("POST", "/api/sunshine/prepare", {}, function (data) {
            if (onSuccess) {
                onSuccess(data);
            }
        });
    }

    function refreshDiagnostics() {
        diagnosticsLoading = true;
        call("GET", "/api/diagnostics", null, function (data) {
            diagnostics = data.report || {};
            diagnosticsLoading = false;
        }, function (message) {
            diagnosticsLoading = false;
            lastError = message;
        });
    }

    function refreshInstall() {
        call("GET", "/api/install", null, function (data) {
            install = data.install || {};
            installRunning = !!(install.status && install.status.running);
        });
    }

    function installDependencies() {
        installPolling = true;
        call("POST", "/api/install/dependencies", {}, function (data) {
            install = data.install || install;
            installRunning = true;
            refreshInstall();
        }, function () {
            installPolling = false;
            installRunning = false;
        });
    }

    onInstallRunningChanged: {
        if (installPolling && !installRunning) {
            installPolling = false;
            refreshDiagnostics();
        }
    }

    /// Format a Unix timestamp as a short relative time.
    function relativeTime(timestamp) {
        if (!timestamp) {
            return "never";
        }
        var seconds = Math.floor(Date.now() / 1000) - timestamp;
        if (seconds < 60) {
            return "just now";
        }
        if (seconds < 3600) {
            return Math.floor(seconds / 60) + " min ago";
        }
        if (seconds < 86400) {
            return Math.floor(seconds / 3600) + " h ago";
        }
        return Math.floor(seconds / 86400) + " d ago";
    }

    function modeLabel(mode) {
        if (mode === "mirror") {
            return "Mirror";
        }
        if (mode === "extend") {
            return "Extend";
        }
        if (mode === "headless") {
            return "Headless";
        }
        return mode || "Unknown";
    }

    function phaseLabel(phase) {
        var labels = {
            "idle": "Idle",
            "preparing": "Preparing",
            "creating_display": "Creating virtual display",
            "configuring_display": "Configuring display",
            "starting_sunshine": "Starting Sunshine",
            "pairing": "Pairing Moonlight",
            "launching": "Launching Moonlight",
            "streaming": "Streaming",
            "stopping": "Stopping",
            "failed": "Failed"
        };
        return labels[phase] || phase || "Idle";
    }

    function defaultAudioFor(mode) {
        var audio = config && config.audio ? config.audio : {};
        if (mode === "extend") {
            return audio.default_on_extend === true;
        }
        if (mode === "headless") {
            return audio.default_on_headless === true;
        }
        return audio.default_on_mirror !== false;
    }

    property Timer statusTimer: Timer {
        interval: 1200
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: backend.refreshStatus()
    }

    property Timer peerTimer: Timer {
        interval: 4000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: {
            backend.refreshPeers();
            backend.refreshPending();
        }
    }

    property Timer installTimer: Timer {
        interval: 1200
        repeat: true
        running: backend.installPolling
        onTriggered: backend.refreshInstall()
    }
}
