import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page

    property var backend
    property var navigate

    title: "Settings"

    padding: 0

    function load() {
        var config = page.backend.config || {};
        var device = config.device || {};
        var display = config.display || {};
        var streaming = config.streaming || {};
        var audio = config.audio || {};
        var control = config.control || {};
        var api = config.api || {};
        var sunshine = config.sunshine || {};

        deviceName.text = device.name || "";
        virtualName.text = display.virtual_name || "KDEFusion";
        physicalOutput.text = display.physical_output || "";
        extendPosition.currentIndex = Math.max(0, extendPosition.model.indexOf(display.extend_position || "right"));
        restoreOnStop.checked = display.restore_on_stop !== false;

        fpsSpin.value = streaming.fps || 60;
        bitrateSpin.value = streaming.bitrate_kbps || 40000;
        codecCombo.currentIndex = Math.max(0, codecCombo.model.indexOf(streaming.codec || "auto"));
        resolutionField.text = streaming.resolution || "";

        audioMirror.checked = audio.default_on_mirror !== false;
        audioExtend.checked = audio.default_on_extend === true;
        audioHeadless.checked = audio.default_on_headless !== false;

        controlEnabled.checked = control.enabled !== false;
        controlPort.value = control.port || 47811;
        apiPort.value = api.port || 47812;

        sunshineHost.text = sunshine.host || "127.0.0.1";
        sunshinePort.value = sunshine.port || 47990;
        sunshineUsername.text = sunshine.username || "kdefusion";
        sunshineManage.checked = sunshine.manage !== false;
        sunshinePassword.text = "";
    }

    function save() {
        var patch = {
            "device": {
                "name": deviceName.text.trim()
            },
            "display": {
                "virtual_name": virtualName.text.trim(),
                "physical_output": physicalOutput.text.trim(),
                "extend_position": extendPosition.currentText,
                "restore_on_stop": restoreOnStop.checked
            },
            "streaming": {
                "fps": fpsSpin.value,
                "bitrate_kbps": bitrateSpin.value,
                "codec": codecCombo.currentText,
                "resolution": resolutionField.text.trim()
            },
            "audio": {
                "default_on_mirror": audioMirror.checked,
                "default_on_extend": audioExtend.checked,
                "default_on_headless": audioHeadless.checked
            },
            "control": {
                "enabled": controlEnabled.checked,
                "port": controlPort.value
            },
            "api": {
                "port": apiPort.value
            },
            "sunshine": {
                "host": sunshineHost.text.trim(),
                "port": sunshinePort.value,
                "username": sunshineUsername.text.trim(),
                "manage": sunshineManage.checked
            }
        };
        page.backend.saveConfig(patch, function (data) {
            applicationWindow().showPassiveNotification(data.detail || "Configuration saved.", "short");
        });
    }

    Connections {
        target: page.backend
        function onConfigChanged() {
            page.load();
        }
    }

    Component.onCompleted: page.load()

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

            Kirigami.Heading { level: 2; text: "This PC" }

            Kirigami.FormLayout {
                Layout.fillWidth: true

                Controls.TextField {
                    id: deviceName
                    Kirigami.FormData.label: "Name:"
                    Layout.fillWidth: true
                    placeholderText: "Used in pairing and notifications"
                }

                Controls.TextField {
                    id: virtualName
                    Kirigami.FormData.label: "Virtual output:"
                    Layout.fillWidth: true
                    placeholderText: "KDEFusion"
                }

                Controls.Label {
                    Kirigami.FormData.label: ""
                    text: "KWin exposes this as Virtual-" + virtualName.text + ". Streams always use this virtual output."
                    color: Kirigami.Theme.disabledTextColor
                    font.pointSize: Kirigami.Theme.smallFont.pointSize
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }

                Controls.TextField {
                    id: physicalOutput
                    Kirigami.FormData.label: "Physical output:"
                    Layout.fillWidth: true
                    placeholderText: "Automatic (for example eDP-1 or HDMI-1)"
                }
            }

            Kirigami.Heading { level: 2; text: "Display" }

            Kirigami.FormLayout {
                Layout.fillWidth: true

                Controls.ComboBox {
                    id: extendPosition
                    Kirigami.FormData.label: "Extend position:"
                    Layout.fillWidth: true
                    model: ["right", "left", "top", "bottom"]
                }

                Controls.CheckBox {
                    id: restoreOnStop
                    Kirigami.FormData.label: ""
                    text: "Restore the previous display layout when a session ends"
                    checked: true
                }
            }

            Kirigami.Heading { level: 2; text: "Streaming" }

            Kirigami.FormLayout {
                Layout.fillWidth: true

                Controls.SpinBox {
                    id: fpsSpin
                    Kirigami.FormData.label: "FPS:"
                    from: 10
                    to: 240
                    stepSize: 10
                    value: 60
                }

                Controls.SpinBox {
                    id: bitrateSpin
                    Kirigami.FormData.label: "Bitrate (Kbps):"
                    from: 500
                    to: 200000
                    stepSize: 1000
                    value: 40000
                }

                Controls.ComboBox {
                    id: codecCombo
                    Kirigami.FormData.label: "Codec:"
                    Layout.fillWidth: true
                    model: ["auto", "H.264", "HEVC", "AV1"]
                }

                Controls.TextField {
                    id: resolutionField
                    Kirigami.FormData.label: "Resolution:"
                    Layout.fillWidth: true
                    placeholderText: "Auto (client native), for example 1920x1080"
                }
            }

            Kirigami.Heading { level: 2; text: "Audio" }

            Kirigami.FormLayout {
                Layout.fillWidth: true

                Controls.CheckBox {
                    id: audioMirror
                    Kirigami.FormData.label: "Defaults:"
                    text: "Stream audio when mirroring"
                    checked: true
                }

                Controls.CheckBox {
                    id: audioExtend
                    Kirigami.FormData.label: ""
                    text: "Stream audio when extending"
                }

                Controls.CheckBox {
                    id: audioHeadless
                    Kirigami.FormData.label: ""
                    text: "Stream audio for a headless display"
                    checked: true
                }
            }

            Controls.Label {
                text: "In this version the host keeps its local audio. The stream carries audio only when the toggle is on."
                color: Kirigami.Theme.disabledTextColor
                font.pointSize: Kirigami.Theme.smallFont.pointSize
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }

            Kirigami.Heading { level: 2; text: "Sunshine" }

            Kirigami.FormLayout {
                Layout.fillWidth: true

                Controls.TextField {
                    id: sunshineHost
                    Kirigami.FormData.label: "Host:"
                    Layout.fillWidth: true
                    placeholderText: "127.0.0.1"
                }

                Controls.SpinBox {
                    id: sunshinePort
                    Kirigami.FormData.label: "Port:"
                    from: 1024
                    to: 65535
                    value: 47990
                }

                Controls.TextField {
                    id: sunshineUsername
                    Kirigami.FormData.label: "Admin user:"
                    Layout.fillWidth: true
                    placeholderText: "kdefusion"
                }

                Controls.CheckBox {
                    id: sunshineManage
                    Kirigami.FormData.label: ""
                    text: "Let KDE Fusion start and configure the Sunshine user service"
                    checked: true
                }

                Controls.TextField {
                    id: sunshinePassword
                    Kirigami.FormData.label: "Admin password:"
                    Layout.fillWidth: true
                    echoMode: TextInput.Password
                    placeholderText: "Stored in KWallet / Secret Service"
                }

                Controls.Button {
                    Kirigami.FormData.label: ""
                    text: "Verify and save Sunshine credentials"
                    icon.name: "dialog-password"
                    onClicked: {
                        page.backend.saveSunshineCredentials(
                            sunshineUsername.text.trim(),
                            sunshinePassword.text,
                            function (data) {
                                sunshinePassword.text = "";
                                applicationWindow().showPassiveNotification(data.detail || "Credentials saved.", "short");
                            });
                    }
                }

                Controls.Button {
                    Kirigami.FormData.label: ""
                    text: "Write Sunshine configuration and Desktop app"
                    icon.name: "document-save"
                    onClicked: {
                        page.backend.prepareSunshine(function () {
                            applicationWindow().showPassiveNotification("Sunshine configuration is ready.", "short");
                        });
                    }
                }
            }

            Kirigami.Heading { level: 2; text: "Network" }

            Kirigami.FormLayout {
                Layout.fillWidth: true

                Controls.CheckBox {
                    id: controlEnabled
                    Kirigami.FormData.label: "Control channel:"
                    text: "Accept sessions from paired PCs on the LAN"
                    checked: true
                }

                Controls.SpinBox {
                    id: controlPort
                    Kirigami.FormData.label: "Control port:"
                    from: 1024
                    to: 65535
                    value: 47811
                }

                Controls.SpinBox {
                    id: apiPort
                    Kirigami.FormData.label: "Local API port:"
                    from: 1024
                    to: 65535
                    value: 47812
                }

                Controls.Label {
                    Kirigami.FormData.label: ""
                    text: "Changing ports or the device name takes effect after restarting kdefusiond."
                    color: Kirigami.Theme.disabledTextColor
                    font.pointSize: Kirigami.Theme.smallFont.pointSize
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }
            }

            RowLayout {
                Layout.fillWidth: true
                Controls.Button {
                    text: "Save settings"
                    icon.name: "document-save"
                    onClicked: page.save()
                }
                Controls.Button {
                    text: "Reload"
                    icon.name: "view-refresh"
                    flat: true
                    onClicked: {
                        page.backend.refreshConfig();
                        page.load();
                    }
                }
                Item { Layout.fillWidth: true }
            }

            Item { Layout.preferredHeight: Kirigami.Units.largeSpacing }
        }
    }
}
