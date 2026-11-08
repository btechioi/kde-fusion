import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid 2.0
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.extras as PlasmaExtras
import org.kde.plasma.components as PlasmaComponents3
import org.kde.kirigami as Kirigami

PlasmoidItem {
    id: root

    width: 340
    height: 460

    Plasmoid.title: "KDE Fusion"
    Plasmoid.icon: "kdefusion"
    Plasmoid.status: Plasmoid.ActiveStatus

    // State
    property bool daemonConnected: false
    property string deviceName: "KDE Fusion"
    property var pairedPeers: []
    property bool sessionActive: false
    property string sessionDetail: ""
    property string sessionPeer: ""

    function apiBase() { return "http://127.0.0.1:47812"; }

    function refresh() {
        // health
        var h = new XMLHttpRequest();
        h.onreadystatechange = function() {
            if (h.readyState === XMLHttpRequest.DONE) {
                if (h.status === 200) {
                    try {
                        var d = JSON.parse(h.responseText);
                        root.daemonConnected = true;
                        root.deviceName = (d.device && d.device.name) ? d.device.name : "KDE Fusion";
                        if (d.session) {
                            root.sessionActive = d.session.phase === "streaming" || d.session.phase === "starting_sunshine" || d.session.phase === "launching";
                            root.sessionDetail = d.session.detail || "";
                            root.sessionPeer = d.session.peer_name || "";
                        } else {
                            root.sessionActive = false;
                            root.sessionDetail = "";
                            root.sessionPeer = "";
                        }
                    } catch (e) { root.daemonConnected = false; }
                } else {
                    root.daemonConnected = false;
                }
            }
        };
        h.open("GET", root.apiBase() + "/api/health");
        h.send();

        // peers
        var p = new XMLHttpRequest();
        p.onreadystatechange = function() {
            if (p.readyState === XMLHttpRequest.DONE && p.status === 200) {
                try {
                    var d = JSON.parse(p.responseText);
                    root.pairedPeers = d.peers || [];
                } catch (e) {}
            }
        };
        p.open("GET", root.apiBase() + "/api/peers");
        p.send();
    }

    function startSession(peerId, mode) {
        var xhr = new XMLHttpRequest();
        xhr.onreadystatechange = function() {
            if (xhr.readyState === XMLHttpRequest.DONE) {
                root.refresh();
            }
        };
        xhr.open("POST", root.apiBase() + "/api/session/start");
        xhr.setRequestHeader("Content-Type", "application/json");
        xhr.send(JSON.stringify({ peer_id: peerId, mode: mode, layout: "right", audio: false, resolution: "", fps: 60, bitrate_kbps: 20000, codec: "", display_mode: "fullscreen" }));
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

    Plasmoid.fullRepresentation: PlasmaExtras.Representation {
        header: PlasmaExtras.Header {
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
                PlasmaComponents3.ToolButton {
                    icon.name: "window-pin"
                }
            }
        }

        // Active session view
        ColumnLayout {
            anchors.fill: parent
            anchors.margins: Kirigami.Units.largeSpacing
            visible: root.sessionActive
            spacing: Kirigami.Units.largeSpacing

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

            PlasmaComponents3.Button {
                Layout.fillWidth: true
                icon.name: "window-new"
                text: "Open KDE Fusion"
                onClicked: Qt.openUrlExternally("http://127.0.0.1:47812")
            }
        }

        // Empty state (no paired devices)
        ColumnLayout {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.largeSpacing * 4
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

            ColumnLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                PlasmaComponents3.Button {
                    Layout.fillWidth: true
                    text: "Install from GitHub"
                    onClicked: Qt.openUrlExternally("https://github.com/btechioi/kde-fusion/releases")
                }

                PlasmaComponents3.Button {
                    Layout.fillWidth: true
                    text: "Read the setup guide"
                    onClicked: Qt.openUrlExternally("https://github.com/btechioi/kde-fusion/blob/main/docs/SETUP.md")
                }
            }
        }

        // Device list (paired peers)
        ColumnLayout {
            anchors.fill: parent
            anchors.margins: Kirigami.Units.largeSpacing
            visible: root.pairedPeers.length > 0 && !root.sessionActive
            spacing: Kirigami.Units.largeSpacing

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
                        PlasmaComponents3.ToolButton {
                            icon.name: "view-split-left-right"
                            onClicked: root.startSession(modelData.device_id, "extend")
                        }
                    }
                }
            }
        }
    }
}