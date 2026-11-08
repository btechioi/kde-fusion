pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid 2.0
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.extras as PlasmaExtras
import org.kde.plasma.components as PlasmaComponents3
import org.kde.kirigami as Kirigami

PlasmoidItem {
    id: root

    width: 360
    height: 520

    Plasmoid.title: "KDE Fusion"
    Plasmoid.icon: "kdefusion"
    Plasmoid.status: PlasmaCore.Types.ActiveStatus

    // State
    property bool daemonConnected: false
    property string deviceName: "KDE Fusion"
    property var pairedPeers: []
    property bool sessionActive: false
    property string sessionDetail: ""
    property string sessionPeer: ""
    property string sessionMode: "mirror"
    property string sessionPhase: "idle"

    // Form state
    property string selMode: "mirror"
    property string selDisplayMode: "fullscreen"
    property bool selAudio: true
    property int selLayoutIndex: 1  // 0=right 1=left 2=top 3=bottom
    property int selPeerIndex: 0

    function apiBase() { return "http://127.0.0.1:47812"; }

    function refresh() {
        var h = new XMLHttpRequest();
        h.onreadystatechange = function() {
            if (h.readyState === XMLHttpRequest.DONE) {
                if (h.status === 200) {
                    try {
                        var d = JSON.parse(h.responseText);
                        root.daemonConnected = true;
                        root.deviceName = (d.device && d.device.name) ? d.device.name : "KDE Fusion";
                        if (d.session) {
                            root.sessionActive = (d.session.phase === "streaming" || d.session.phase === "starting_sunshine" || d.session.phase === "launching" || d.session.phase === "configuring_display" || d.session.phase === "creating_display");
                            root.sessionDetail = d.session.detail || "";
                            root.sessionPeer = d.session.peer_name || "";
                            root.sessionMode = d.session.mode || "mirror";
                            root.sessionPhase = d.session.phase || "idle";
                        }
                    } catch (e) { root.daemonConnected = false; }
                } else {
                    root.daemonConnected = false;
                }
            }
        };
        h.open("GET", root.apiBase() + "/api/health");
        h.send();

        var p = new XMLHttpRequest();
        p.onreadystatechange = function() {
            if (p.readyState === XMLHttpRequest.DONE && p.status === 200) {
                try {
                    var d = JSON.parse(p.responseText);
                    root.pairedPeers = d.peers || [];
                    if (root.selPeerIndex >= root.pairedPeers.length) root.selPeerIndex = 0;
                } catch (e) {}
            }
        };
        p.open("GET", root.apiBase() + "/api/peers");
        p.send();

        var s = new XMLHttpRequest();
        s.onreadystatechange = function() {
            if (s.readyState === XMLHttpRequest.DONE && s.status === 200) {
                try {
                    var d = JSON.parse(s.responseText);
                    if (d.status) {
                        root.sessionActive = (d.status.phase === "streaming" || d.status.phase === "starting_sunshine" || d.status.phase === "launching");
                        root.sessionDetail = d.status.detail || "";
                        root.sessionPeer = d.status.peer_name || "";
                        root.sessionMode = d.status.mode || "mirror";
                        root.sessionPhase = d.status.phase || "idle";
                    }
                } catch (e) {}
            }
        };
        s.open("GET", root.apiBase() + "/api/session/status");
        s.send();
    }

    function startSession(peerId, mode) {
        if (peerId) {
            // find index
            for (var i = 0; i < root.pairedPeers.length; i++) {
                if (root.pairedPeers[i].device_id === peerId) { root.selPeerIndex = i; break; }
            }
        }
        if (mode) root.selMode = mode;
        if (root.pairedPeers.length === 0) return;
        var peer = root.pairedPeers[root.selPeerIndex];
        var layout = ["right","left","top","bottom"][root.selLayoutIndex];
        var res = (root.selDisplayMode === "windowed-720p") ? "1280x720" : "";
        var xhr = new XMLHttpRequest();
        xhr.onreadystatechange = function() { if (xhr.readyState === XMLHttpRequest.DONE) root.refresh(); };
        xhr.open("POST", root.apiBase() + "/api/session/start");
        xhr.setRequestHeader("Content-Type", "application/json");
        xhr.send(JSON.stringify({
            peer_id: peer.device_id,
            mode: root.selMode,
            display_mode: (root.selDisplayMode === "fullscreen") ? "fullscreen" : "windowed",
            layout: layout,
            audio: root.selAudio,
            resolution: res,
            fps: 60,
            bitrate_kbps: 20000,
            codec: "auto"
        }));
    }

    function stopSession() {
        var xhr = new XMLHttpRequest();
        xhr.onreadystatechange = function() { if (xhr.readyState === XMLHttpRequest.DONE) root.refresh(); };
        xhr.open("POST", root.apiBase() + "/api/session/stop");
        xhr.setRequestHeader("Content-Type", "application/json");
        xhr.send(JSON.stringify({ reason: "stopped from plasmoid" }));
    }

    Timer {
        interval: 3000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: root.refresh()
    }

    fullRepresentation: PlasmaExtras.Representation {
        header: PlasmaExtras.PlasmoidHeading {
            RowLayout {
                anchors.fill: parent
                PlasmaComponents3.ToolButton {
                    icon.name: "go-previous"
                    onClicked: root.Plasmoid.expanded = false
                }
                PlasmaExtras.Heading {
                    Layout.fillWidth: true
                    text: root.deviceName
                    level: 2
                }
                PlasmaComponents3.ToolButton {
                    icon.name: "configure"
                    onClicked: Qt.openUrlExternally("http://127.0.0.1:47812")
                }
            }
        }

        Flickable {
            anchors.fill: parent
            contentHeight: content.implicitHeight + Kirigami.Units.largeSpacing * 2
            clip: true

            ColumnLayout {
                id: content
                width: parent.width
                anchors.margins: Kirigami.Units.largeSpacing
                spacing: Kirigami.Units.largeSpacing

                // Active session view
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: root.sessionActive
                    spacing: Kirigami.Units.smallSpacing

                    PlasmaComponents3.Label {
                        Layout.fillWidth: true
                        text: "Streaming: " + root.sessionDetail
                        font.bold: true
                        wrapMode: Text.WordWrap
                    }

                    PlasmaComponents3.Button {
                        Layout.fillWidth: true
                        icon.name: "media-playback-stop"
                        text: "Stop session"
                        onClicked: root.stopSession()
                    }
                }

                // Empty state
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: root.pairedPeers.length === 0 && !root.sessionActive
                    spacing: Kirigami.Units.largeSpacing

                    Kirigami.Icon {
                        Layout.alignment: Qt.AlignHCenter
                        source: "dialog-cancel"
                        implicitWidth: Kirigami.Units.iconSizes.huge
                        implicitHeight: Kirigami.Units.iconSizes.huge
                    }

                    PlasmaComponents3.Label {
                        Layout.fillWidth: true
                        horizontalAlignment: Text.AlignHCenter
                        text: "No paired devices"
                        font.bold: true
                    }

                    PlasmaComponents3.Button {
                        Layout.alignment: Qt.AlignHCenter
                        icon.name: "list-add"
                        text: "+ Pair a Device..."
                        onClicked: Qt.openUrlExternally("http://127.0.0.1:47812")
                    }
                }

                // New session form
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: root.pairedPeers.length > 0 && !root.sessionActive
                    spacing: Kirigami.Units.smallSpacing

                    PlasmaComponents3.Label {
                        text: "Start a session"
                        font.bold: true
                    }

                    PlasmaComponents3.ComboBox {
                        Layout.fillWidth: true
                        model: root.pairedPeers.map(function(p){ return p.name; })
                        currentIndex: root.selPeerIndex
                        onActivated: function(index) { root.selPeerIndex = index; }
                    }

                    PlasmaComponents3.ComboBox {
                        Layout.fillWidth: true
                        model: ["Mirror", "Extend", "Headless"]
                        currentIndex: root.selMode === "mirror" ? 0 : root.selMode === "extend" ? 1 : 2
                        onActivated: function(index) {
                            root.selMode = (index === 0) ? "mirror" : (index === 1) ? "extend" : "headless";
                        }
                    }

                    PlasmaComponents3.ComboBox {
                        Layout.fillWidth: true
                        visible: root.selMode === "extend"
                        model: ["Right", "Left", "Above", "Below"]
                        currentIndex: root.selLayoutIndex
                        onActivated: function(index) { root.selLayoutIndex = index; }
                    }

                    PlasmaComponents3.ComboBox {
                        Layout.fillWidth: true
                        model: ["Fullscreen", "Windowed 720p", "Windowed (unscaled)"]
                        currentIndex: root.selDisplayMode === "fullscreen" ? 0 : root.selDisplayMode === "windowed-720p" ? 1 : 2
                        onActivated: function(index) {
                            if (index === 0) root.selDisplayMode = "fullscreen";
                            else if (index === 1) root.selDisplayMode = "windowed-720p";
                            else root.selDisplayMode = "windowed-unscaled";
                        }
                    }

                    PlasmaComponents3.Switch {
                        Layout.fillWidth: true
                        text: root.selAudio ? "Audio on" : "Audio off"
                        checked: root.selAudio
                        onToggled: root.selAudio = checked
                    }

                    PlasmaComponents3.Button {
                        Layout.fillWidth: true
                        icon.name: "media-playback-start"
                        text: "Start session"
                        onClicked: root.startSession()
                    }
                }

                // Peer quick list
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: root.pairedPeers.length > 0
                    spacing: Kirigami.Units.smallSpacing

                    PlasmaComponents3.Label {
                        text: "Your PCs"
                        font.bold: true
                    }

                    Repeater {
                        model: root.pairedPeers
                        delegate: PlasmaComponents3.ItemDelegate {
                            required property var modelData
                            Layout.fillWidth: true
                            contentItem: RowLayout {
                                spacing: Kirigami.Units.smallSpacing
                                PlasmaComponents3.Label {
                                    Layout.fillWidth: true
                                    text: modelData.name
                                    elide: Text.ElideRight
                                }
                                PlasmaComponents3.ToolButton {
                                    icon.name: "media-playback-start"
                                    onClicked: root.startSession(modelData.device_id, "mirror")
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}