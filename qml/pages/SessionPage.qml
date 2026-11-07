import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kirigami.primitives as KirigamiPrimitives

Kirigami.Page {
    id: page

    property var backend
    property var navigate
    property string preselectPeerId: ""
    property string preselectMode: "mirror"

    property string mode: "mirror"
    property bool windowed: false
    property bool audio: true
    property bool audioTouched: false
    property bool advanced: false

    title: "Session"

    padding: 0

    function selectedPeer() {
        if (!page.backend.pairedPeers || page.backend.pairedPeers.length === 0) {
            return null;
        }
        var index = peerCombo.currentIndex;
        if (index < 0 || index >= page.backend.pairedPeers.length) {
            index = 0;
        }
        return page.backend.pairedPeers[index];
    }

    function selectPeer(peerId) {
        if (!peerId || !page.backend.pairedPeers) {
            return;
        }
        for (var i = 0; i < page.backend.pairedPeers.length; i++) {
            if (page.backend.pairedPeers[i].device_id === peerId) {
                peerCombo.currentIndex = i;
                return;
            }
        }
    }

    function applyPreselection() {
        if (page.preselectMode) {
            page.mode = page.preselectMode;
        }
        if (page.preselectPeerId) {
            page.selectPeer(page.preselectPeerId);
        }
    }

    function startSession() {
        var peer = page.selectedPeer();
        if (!peer) {
            applicationWindow().showPassiveNotification("Pair a PC first on the Devices page.", "long");
            return;
        }
        var request = {
            peer_id: peer.device_id,
            mode: page.mode,
            display_mode: page.windowed ? "windowed" : "fullscreen",
            layout: layoutCombo.currentIndex === 0 ? "right"
                : layoutCombo.currentIndex === 1 ? "left"
                : layoutCombo.currentIndex === 2 ? "top" : "bottom",
            audio: page.audio,
            resolution: resolutionField.text.trim(),
            fps: fpsSpin.value,
            bitrate_kbps: bitrateSpin.value,
            codec: codecCombo.currentText
        };
        page.backend.startSession(request, function () {
            applicationWindow().showPassiveNotification("Session requested. Look at both screens.", "short");
        });
    }

    Connections {
        target: page.backend
        function onPairedPeersChanged() {
            page.selectPeer(page.preselectPeerId);
        }
    }

    Component.onCompleted: page.applyPreselection()

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
                    : "The KDE Fusion daemon is not reachable."
            }

            // Active session -------------------------------------------------
            Kirigami.Card {
                Layout.fillWidth: true
                visible: page.backend.session.phase !== undefined && page.backend.session.phase !== "idle"

                contentItem: ColumnLayout {
                    spacing: Kirigami.Units.smallSpacing

                    RowLayout {
                        Layout.fillWidth: true
                        KirigamiPrimitives.Icon {
                            source: page.backend.active ? "media-playback-start" : "view-refresh"
                            Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                            Layout.preferredHeight: Kirigami.Units.iconSizes.medium
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            Controls.Label {
                                text: page.backend.phaseLabel(page.backend.session.phase)
                                font.weight: Font.DemiBold
                                font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.2
                            }
                            Controls.Label {
                                text: page.backend.session.detail || ""
                                color: Kirigami.Theme.disabledTextColor
                                elide: Text.ElideRight
                                Layout.fillWidth: true
                            }
                        }
                        Kirigami.Badge {
                            text: page.backend.session.role === "viewer" ? "Viewer" : "Host"
                            visible: page.backend.session.role !== undefined && page.backend.session.role.length > 0
                        }
                    }

                    Controls.ProgressBar {
                        Layout.fillWidth: true
                        indeterminate: page.backend.busy
                        value: page.backend.active ? 1 : 0
                        from: 0
                        to: 1
                    }

                    GridLayout {
                        Layout.fillWidth: true
                        columns: 2
                        columnSpacing: Kirigami.Units.largeSpacing
                        rowSpacing: Kirigami.Units.smallSpacing

                        Controls.Label { text: "Peer"; color: Kirigami.Theme.disabledTextColor }
                        Controls.Label {
                            text: page.backend.session.peer_name || "—"
                            Layout.fillWidth: true
                            elide: Text.ElideRight
                        }

                        Controls.Label { text: "Mode"; color: Kirigami.Theme.disabledTextColor }
                        Controls.Label {
                            text: page.backend.modeLabel(page.backend.session.mode)
                            Layout.fillWidth: true
                        }

                        Controls.Label { text: "Resolution"; color: Kirigami.Theme.disabledTextColor }
                        Controls.Label {
                            text: page.backend.session.width > 0
                                ? page.backend.session.width + " × " + page.backend.session.height
                                : "—"
                            Layout.fillWidth: true
                        }

                        Controls.Label { text: "Audio"; color: Kirigami.Theme.disabledTextColor }
                        Controls.Label {
                            text: page.backend.session.audio ? "On" : "Off"
                            Layout.fillWidth: true
                        }
                    }

                    Kirigami.InlineMessage {
                        Layout.fillWidth: true
                        visible: page.backend.session.error !== undefined
                                 && page.backend.session.error.length > 0
                        type: Kirigami.MessageType.Error
                        text: page.backend.session.error || ""
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Controls.Button {
                            text: "Stop session"
                            icon.name: "media-playback-stop"
                            enabled: page.backend.session.phase !== "stopping"
                            onClicked: page.backend.stopSession("stopped from KDE Fusion")
                        }
                        Item { Layout.fillWidth: true }
                    }
                }
            }

            // New session ----------------------------------------------------
            Kirigami.Heading {
                level: 2
                text: page.backend.active ? "Start another session" : "Start a session"
                visible: !page.backend.busy || page.backend.session.phase === "failed"
            }

            Controls.Label {
                visible: !page.backend.busy || page.backend.session.phase === "failed"
                text: "Choose which PC will receive this desktop. Do not start a session while one is active."
                color: Kirigami.Theme.disabledTextColor
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }

            Kirigami.FormLayout {
                Layout.fillWidth: true
                enabled: !page.backend.busy || page.backend.session.phase === "failed"

                Controls.ComboBox {
                    id: peerCombo
                    Kirigami.FormData.label: "PC:"
                    Layout.fillWidth: true
                    textRole: "name"
                    model: page.backend.pairedPeers || []
                    displayText: count === 0 ? "No paired PCs" : currentText
                }

                Controls.ComboBox {
                    id: modeCombo
                    Kirigami.FormData.label: "Mode:"
                    Layout.fillWidth: true
                    model: ["Mirror this display", "Extend this display", "Headless virtual display"]
                    currentIndex: page.mode === "mirror" ? 0 : page.mode === "extend" ? 1 : 2
                    onActivated: {
                        page.mode = index === 0 ? "mirror" : index === 1 ? "extend" : "headless";
                        if (!page.audioTouched) {
                            page.audio = page.backend.defaultAudioFor(page.mode);
                        }
                    }
                }

                Controls.ComboBox {
                    id: layoutCombo
                    Kirigami.FormData.label: "Extend to:"
                    Layout.fillWidth: true
                    visible: page.mode === "extend"
                    model: ["Right of this screen", "Left of this screen", "Above this screen", "Below this screen"]
                }

                Controls.Switch {
                    Kirigami.FormData.label: "Audio:"
                    text: page.audio ? "Stream system audio" : "No audio"
                    checked: page.audio
                    onToggled: {
                        page.audio = checked;
                        page.audioTouched = true;
                    }
                }

                Controls.Label {
                    Kirigami.FormData.label: ""
                    text: page.mode === "extend"
                        ? "Audio defaults to off for extend to avoid feedback."
                        : "The host keeps its local audio in this version."
                    color: Kirigami.Theme.disabledTextColor
                    font.pointSize: Kirigami.Theme.smallFont.pointSize
                    Layout.fillWidth: true
                }

                Controls.Button {
                    Kirigami.FormData.label: ""
                    text: page.advanced ? "Hide advanced options" : "Advanced options"
                    flat: true
                    onClicked: page.advanced = !page.advanced
                }

                Controls.Switch {
                    Kirigami.FormData.label: "Windowed (720p):"
                    text: checked ? "Moonlight opens in a 1280x720 window" : "Moonlight opens fullscreen"
                    checked: page.windowed
                    onToggled: {
                        page.windowed = checked;
                        if (checked) {
                            resolutionField.text = "1280x720";
                        }
                    }
                    visible: page.advanced
                }

                Controls.SpinBox {
                    id: fpsSpin
                    Kirigami.FormData.label: "FPS:"
                    visible: page.advanced
                    from: 10
                    to: 240
                    stepSize: 10
                    value: page.backend.config.streaming
                        ? page.backend.config.streaming.fps : 60
                }

                Controls.SpinBox {
                    id: bitrateSpin
                    Kirigami.FormData.label: "Bitrate (Kbps):"
                    visible: page.advanced
                    from: 500
                    to: 200000
                    stepSize: 1000
                    value: page.backend.config.streaming
                        ? page.backend.config.streaming.bitrate_kbps : 40000
                }

                Controls.ComboBox {
                    id: codecCombo
                    Kirigami.FormData.label: "Codec:"
                    visible: page.advanced
                    model: ["auto", "H.264", "HEVC", "AV1"]
                    currentIndex: {
                        var requested = page.backend.config.streaming
                            ? page.backend.config.streaming.codec : "auto";
                        var index = model.indexOf(requested);
                        return index < 0 ? 0 : index;
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                Controls.Button {
                    text: "Start session"
                    icon.name: "media-playback-start"
                    enabled: page.selectedPeer() !== null && !page.backend.busy
                    onClicked: page.startSession()
                }
                Controls.Button {
                    text: "Refresh"
                    icon.name: "view-refresh"
                    flat: true
                    onClicked: page.backend.refreshAll()
                }
                Item { Layout.fillWidth: true }
            }

            Item { Layout.preferredHeight: Kirigami.Units.largeSpacing }
        }
    }
}
