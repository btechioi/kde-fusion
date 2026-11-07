import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kirigami.primitives as KirigamiPrimitives

import "../components" as Components

Kirigami.Page {
    id: page

    property var backend
    property var navigate

    signal navigateToSession(string peerId, string mode)

    title: "Devices"

    actions: [
        Kirigami.Action {
            text: "Refresh"
            icon.name: "view-refresh"
            onTriggered: page.backend.refreshAll()
        }
    ]

    padding: 0

    Flickable {
        id: flick
        anchors.fill: parent
        clip: true
        contentWidth: width
        contentHeight: content.implicitHeight + Kirigami.Units.largeSpacing * 2
        boundsBehavior: Flickable.StopAtBounds

        Controls.ScrollBar.vertical: Controls.ScrollBar { }

        ColumnLayout {
            id: content
            x: Kirigami.Units.largeSpacing
            y: Kirigami.Units.largeSpacing
            width: flick.width - Kirigami.Units.largeSpacing * 2
            spacing: Kirigami.Units.largeSpacing

            Kirigami.InlineMessage {
                Layout.fillWidth: true
                visible: !page.backend.connected
                type: Kirigami.MessageType.Error
                text: page.backend.lastError.length > 0
                    ? page.backend.lastError
                    : "The KDE Fusion daemon is not reachable. Start it with: systemctl --user start kdefusiond"
            }

            Kirigami.InlineMessage {
                Layout.fillWidth: true
                visible: (page.backend.installMissing || []).length > 0
                type: Kirigami.MessageType.Warning
                text: "Sunshine, Moonlight or the KDE virtual monitor is not installed. "
                      + "Install the streaming packages before starting a session."
                actions: [
                    Kirigami.Action {
                        text: "Open Diagnostics"
                        icon.name: "system-software-update"
                        onTriggered: {
                            if (page.navigate) {
                                page.navigate("diagnostics");
                            }
                        }
                    }
                ]
            }

            Kirigami.Card {
                Layout.fillWidth: true
                contentItem: ColumnLayout {
                    spacing: Kirigami.Units.smallSpacing

                    RowLayout {
                        Layout.fillWidth: true
                        KirigamiPrimitives.Icon {
                            source: "computer"
                            Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                            Layout.preferredHeight: Kirigami.Units.iconSizes.medium
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            Controls.Label {
                                text: page.backend.deviceName || "This PC"
                                font.weight: Font.DemiBold
                            }
                            Controls.Label {
                                text: "Device ID: " + (page.backend.deviceId || "unknown")
                                color: Kirigami.Theme.disabledTextColor
                                font.pointSize: Kirigami.Theme.smallFont.pointSize
                                elide: Text.ElideMiddle
                                Layout.fillWidth: true
                            }
                        }
                        Kirigami.Badge {
                            text: page.backend.connected ? "Daemon online" : "Daemon offline"
                            customColor: page.backend.connected
                                ? Kirigami.Theme.positiveTextColor
                                : Kirigami.Theme.negativeTextColor
                        }
                    }

                    Controls.Label {
                        Layout.fillWidth: true
                        wrapMode: Text.WordWrap
                        text: "Mirror this desktop to another PC, extend it as a second screen, " +
                              "or stream a headless virtual display. The other PC needs KDE Fusion too."
                        color: Kirigami.Theme.disabledTextColor
                    }
                }
            }

            Kirigami.Heading {
                level: 3
                text: "Pairing requests"
                visible: (page.backend.pending || []).length > 0
            }

            Repeater {
                model: page.backend.pending || []
                delegate: Kirigami.Card {
                    required property var modelData
                    Layout.fillWidth: true
                    contentItem: ColumnLayout {
                        spacing: Kirigami.Units.smallSpacing

                        Controls.Label {
                            text: modelData.name + " wants to pair"
                            font.weight: Font.DemiBold
                            Layout.fillWidth: true
                        }
                        Controls.Label {
                            text: "Device ID " + modelData.device_id + "  ·  " + modelData.remote_addr
                            color: Kirigami.Theme.disabledTextColor
                            font.pointSize: Kirigami.Theme.smallFont.pointSize
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                        Controls.Label {
                            text: "Ask the other PC to confirm code: " + modelData.code
                            wrapMode: Text.WordWrap
                            Layout.fillWidth: true
                        }
                        RowLayout {
                            Layout.fillWidth: true
                            Controls.Button {
                                text: "Approve"
                                icon.name: "dialog-ok"
                                onClicked: page.backend.approve(modelData.id)
                            }
                            Controls.Button {
                                text: "Reject"
                                icon.name: "dialog-cancel"
                                onClicked: page.backend.reject(modelData.id)
                            }
                            Item { Layout.fillWidth: true }
                        }
                    }
                }
            }

            Kirigami.Heading {
                level: 3
                text: "Your PCs"
            }

            Controls.Label {
                visible: (page.backend.pairedPeers || []).length === 0
                text: "No paired PCs yet. Turn on KDE Fusion on the other PC; it will appear below over mDNS."
                color: Kirigami.Theme.disabledTextColor
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }

            Repeater {
                model: page.backend.pairedPeers || []
                delegate: Components.PeerCard {
                    required property var modelData
                    Layout.fillWidth: true
                    peer: modelData
                    onStartRequested: function (mode) {
                        page.navigateToSession(modelData.device_id, mode);
                    }
                    onRemoveRequested: {
                        removeDialog.peerId = modelData.device_id;
                        removeDialog.peerName = modelData.name;
                        removeDialog.open();
                    }
                }
            }

            Kirigami.Heading {
                level: 3
                text: "Nearby"
                visible: (page.backend.nearbyPeers || []).length > 0
            }

            Repeater {
                model: page.backend.nearbyPeers || []
                delegate: Components.PeerCard {
                    required property var modelData
                    Layout.fillWidth: true
                    peer: modelData
                    onPairRequested: page.backend.pair(modelData.device_id, "")
                }
            }

            Item { Layout.preferredHeight: Kirigami.Units.largeSpacing }
        }
    }

    Controls.Dialog {
        id: pairDialog
        modal: true
        title: "Pair with " + page.backend.pairingPeerName
        standardButtons: Controls.Dialog.Ok
        visible: page.backend.pairingDialogVisible
        anchors.centerIn: parent
        onAccepted: page.backend.pairingDialogVisible = false
        onClosed: page.backend.pairingDialogVisible = false
        contentItem: ColumnLayout {
            spacing: Kirigami.Units.smallSpacing
            Controls.Label {
                text: "A pairing request was sent. Approve it on the other PC."
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
            Controls.Label {
                text: "Code: " + page.backend.pairingCode
                font.pointSize: Kirigami.Theme.defaultFont.pointSize * 2
                font.weight: Font.Bold
                Layout.alignment: Qt.AlignHCenter
            }
            Controls.Label {
                text: "Both screens should show the same code. The peer appears under “Your PCs” once approved."
                wrapMode: Text.WordWrap
                color: Kirigami.Theme.disabledTextColor
                Layout.fillWidth: true
            }
        }
    }

    Controls.Dialog {
        id: removeDialog
        property string peerId: ""
        property string peerName: ""
        modal: true
        title: "Remove " + peerName + "?"
        standardButtons: Controls.Dialog.Cancel | Controls.Dialog.Ok
        anchors.centerIn: parent
        contentItem: Controls.Label {
            text: "This revokes the pairing token. The other PC must pair again before it can start sessions."
            wrapMode: Text.WordWrap
        }
        onAccepted: page.backend.removePeer(peerId)
    }
}
