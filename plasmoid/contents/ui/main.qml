import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid 2.0
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.extras as PlasmaExtras
import org.kde.plasma.components as PlasmaComponents3
import org.kde.kirigami as Kirigami

PlasmoidItem {
    id: root

    width: 320
    height: 420

    Plasmoid.title: "KDE Fusion"
    Plasmoid.icon: "kdefusion"

    // Status data refreshed from the daemon
    property bool daemonConnected: false
    property string deviceName: ""
    property var pairedPeers: []
    property bool active: false
    property string detail: ""

    function refreshStatus() {
        var xhr = new XMLHttpRequest();
        xhr.onreadystatechange = function() {
            if (xhr.readyState === XMLHttpRequest.DONE) {
                if (xhr.status === 200) {
                    try {
                        var data = JSON.parse(xhr.responseText);
                        root.daemonConnected = true;
                        root.deviceName = data.device ? data.device.name : "";
                        root.pairedPeers = (data.session ? [] : []);
                        // peer list comes from /api/peers
                    } catch (e) {}
                } else {
                    root.daemonConnected = false;
                }
            }
        };
        xhr.open("GET", "http://127.0.0.1:47812/api/health");
        xhr.send();
    }

    Timer {
        interval: 3000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: root.refreshStatus()
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
                    text: "KDE Fusion"
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

        // Empty state shown when no paired devices
        ColumnLayout {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.largeSpacing * 4
            visible: root.pairedPeers.length === 0
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
                    text: "Install from Google Play"
                    onClicked: Qt.openUrlExternally("https://github.com/btechioi/kde-fusion/releases")
                }

                PlasmaComponents3.Button {
                    Layout.fillWidth: true
                    text: "Install from F-Droid"
                    onClicked: Qt.openUrlExternally("https://github.com/btechioi/kde-fusion/blob/main/docs/SETUP.md")
                }
            }
        }
    }
}