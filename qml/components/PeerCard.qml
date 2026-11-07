import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kirigami.primitives as KirigamiPrimitives

/// One PC in the devices list, either already paired or merely discovered.
Kirigami.Card {
    id: card

    property var peer
    property bool compact: false

    signal pairRequested()
    signal startRequested(string mode)
    signal removeRequested()

    contentItem: ColumnLayout {
        spacing: Kirigami.Units.smallSpacing

        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.largeSpacing

            KirigamiPrimitives.Icon {
                source: card.peer && card.peer.paired ? "computer" : "network-wired"
                Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                Layout.preferredHeight: Kirigami.Units.iconSizes.medium
                color: card.peer && card.peer.online
                    ? Kirigami.Theme.positiveTextColor
                    : Kirigami.Theme.disabledTextColor
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0

                Controls.Label {
                    text: card.peer ? card.peer.name : ""
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }

                Controls.Label {
                    text: {
                        if (!card.peer) {
                            return "";
                        }
                        var address = (card.peer.addresses || []).join(", ");
                        if (!address) {
                            address = "No address";
                        }
                        return address + "  ·  " + (card.peer.online ? "Online" : "Offline");
                    }
                    color: Kirigami.Theme.disabledTextColor
                    font.pointSize: Kirigami.Theme.smallFont.pointSize
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
            }

            Kirigami.Badge {
                visible: card.peer && card.peer.online
                text: "Online"
                customColor: Kirigami.Theme.positiveTextColor
            }
        }

        Controls.Label {
            visible: card.peer && card.peer.paired && card.peer.sunshine_paired === false
            text: "Moonlight is not paired with Sunshine yet. Start a session to pair automatically."
            wrapMode: Text.WordWrap
            color: Kirigami.Theme.neutralTextColor
            font.pointSize: Kirigami.Theme.smallFont.pointSize
            Layout.fillWidth: true
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            Controls.Button {
                visible: card.peer && !card.peer.paired
                text: "Pair"
                icon.name: "list-add"
                enabled: card.peer && card.peer.online
                onClicked: card.pairRequested()
            }

            Controls.Button {
                visible: card.peer && card.peer.paired
                text: "Mirror"
                icon.name: "media-playback-start"
                onClicked: card.startRequested("mirror")
            }

            Controls.Button {
                visible: card.peer && card.peer.paired
                text: "Extend"
                icon.name: "view-fullscreen"
                onClicked: card.startRequested("extend")
            }

            Controls.Button {
                visible: card.peer && card.peer.paired
                text: "Headless"
                icon.name: "preferences-desktop-display"
                onClicked: card.startRequested("headless")
            }

            Item { Layout.fillWidth: true }

            Controls.Button {
                visible: card.peer && card.peer.paired
                text: "Remove"
                icon.name: "edit-delete"
                flat: true
                onClicked: card.removeRequested()
            }
        }
    }
}
