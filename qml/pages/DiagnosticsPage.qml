import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kirigami.primitives as KirigamiPrimitives

Kirigami.Page {
    id: page

    property var backend
    property var navigate

    title: "Diagnostics"

    padding: 0

    function statusColor(status) {
        if (status === "ok") {
            return Kirigami.Theme.positiveTextColor;
        }
        if (status === "warn") {
            return Kirigami.Theme.neutralTextColor;
        }
        if (status === "error") {
            return Kirigami.Theme.negativeTextColor;
        }
        return Kirigami.Theme.disabledTextColor;
    }

    function statusLabel(status) {
        if (status === "ok") {
            return "OK";
        }
        if (status === "warn") {
            return "Warning";
        }
        if (status === "error") {
            return "Error";
        }
        return "Info";
    }

    Component.onCompleted: {
        if (!page.backend.diagnostics || !page.backend.diagnostics.checks) {
            page.backend.refreshDiagnostics();
        }
        page.backend.refreshInstall();
    }

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

            RowLayout {
                Layout.fillWidth: true
                Controls.Button {
                    text: page.backend.diagnosticsLoading ? "Checking…" : "Run checks"
                    icon.name: "view-refresh"
                    enabled: !page.backend.diagnosticsLoading
                    onClicked: page.backend.refreshDiagnostics()
                }
                Item { Layout.fillWidth: true }
                Controls.Label {
                    visible: page.backend.diagnostics.generated_at !== undefined
                    text: page.backend.diagnostics.generated_at
                        ? "Last run: " + new Date(page.backend.diagnostics.generated_at * 1000).toLocaleString()
                        : ""
                    color: Kirigami.Theme.disabledTextColor
                    font.pointSize: Kirigami.Theme.smallFont.pointSize
                }
            }

            Kirigami.Card {
                Layout.fillWidth: true
                visible: page.backend.installPackages.length > 0
                contentItem: ColumnLayout {
                    spacing: Kirigami.Units.smallSpacing

                    RowLayout {
                        Layout.fillWidth: true
                        KirigamiPrimitives.Icon {
                            source: page.backend.installMissing.length > 0
                                ? "system-software-update" : "dialog-ok"
                            color: page.backend.installMissing.length > 0
                                ? Kirigami.Theme.neutralTextColor
                                : Kirigami.Theme.positiveTextColor
                            Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                            Layout.preferredHeight: Kirigami.Units.iconSizes.medium
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            Controls.Label {
                                text: page.backend.installRunning
                                    ? "Installing streaming packages"
                                    : page.backend.installMissing.length > 0
                                        ? "Streaming packages are missing"
                                        : "Streaming packages are installed"
                                font.weight: Font.DemiBold
                            }
                            Controls.Label {
                                text: page.backend.installRunning
                                    ? "Polkit may ask for your administrator password. Do not close KDE Fusion."
                                    : page.backend.installMissing.length > 0
                                        ? "The Sunshine host, the Moonlight viewer and the KDE virtual monitor are required."
                                        : "Sunshine, Moonlight and krfb are ready to use."
                                color: Kirigami.Theme.disabledTextColor
                                wrapMode: Text.WordWrap
                                Layout.fillWidth: true
                            }
                        }
                        Controls.BusyIndicator {
                            visible: page.backend.installRunning
                            running: visible
                            Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                            Layout.preferredHeight: Kirigami.Units.iconSizes.medium
                        }
                        Kirigami.Badge {
                            visible: !page.backend.installRunning
                            text: page.backend.installMissing.length > 0 ? "Missing" : "Ready"
                            customColor: page.backend.installMissing.length > 0
                                ? Kirigami.Theme.negativeTextColor
                                : Kirigami.Theme.positiveTextColor
                        }
                    }

                    Controls.Label {
                        visible: page.backend.install.distro !== undefined
                                 && String(page.backend.install.distro).length > 0
                        text: page.backend.install.distro
                              + (page.backend.install.manager
                                 ? "  ·  " + page.backend.install.manager : "")
                        color: Kirigami.Theme.disabledTextColor
                        font.pointSize: Kirigami.Theme.smallFont.pointSize
                        Layout.fillWidth: true
                    }

                    Repeater {
                        model: page.backend.installPackages
                        delegate: RowLayout {
                            required property var modelData
                            Layout.fillWidth: true
                            Layout.leftMargin: Kirigami.Units.iconSizes.smallMedium + Kirigami.Units.smallSpacing

                            KirigamiPrimitives.Icon {
                                source: modelData.installed ? "dialog-ok" : "data-error"
                                color: modelData.installed
                                    ? Kirigami.Theme.positiveTextColor
                                    : Kirigami.Theme.negativeTextColor
                                Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                                Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                            }
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 0
                                Controls.Label {
                                    text: modelData.label + "  ·  " + modelData.package
                                        + (modelData.installed ? "" : "  ·  " + modelData.method)
                                    Layout.fillWidth: true
                                    elide: Text.ElideRight
                                }
                                Controls.Label {
                                    text: modelData.purpose
                                    color: Kirigami.Theme.disabledTextColor
                                    font.pointSize: Kirigami.Theme.smallFont.pointSize
                                    wrapMode: Text.WordWrap
                                    Layout.fillWidth: true
                                }
                            }
                            Controls.Label {
                                text: modelData.installed ? "Installed" : "Not installed"
                                color: modelData.installed
                                    ? Kirigami.Theme.positiveTextColor
                                    : Kirigami.Theme.negativeTextColor
                                font.pointSize: Kirigami.Theme.smallFont.pointSize
                            }
                        }
                    }

                    Repeater {
                        model: page.backend.installManual
                        delegate: ColumnLayout {
                            required property var modelData
                            Layout.fillWidth: true
                            Layout.leftMargin: Kirigami.Units.iconSizes.smallMedium + Kirigami.Units.smallSpacing
                            spacing: Kirigami.Units.smallSpacing

                            Controls.Label {
                                text: modelData.label + " needs a manual step"
                                font.weight: Font.DemiBold
                                Layout.fillWidth: true
                            }
                            Controls.Label {
                                text: modelData.note
                                color: Kirigami.Theme.disabledTextColor
                                font.pointSize: Kirigami.Theme.smallFont.pointSize
                                wrapMode: Text.WordWrap
                                Layout.fillWidth: true
                            }
                            RowLayout {
                                Layout.fillWidth: true
                                Controls.Label {
                                    text: modelData.command
                                    font.family: "monospace"
                                    font.pointSize: Kirigami.Theme.smallFont.pointSize
                                    wrapMode: Text.WordWrap
                                    Layout.fillWidth: true
                                }
                                Controls.Button {
                                    text: "Open"
                                    icon.name: "internet-services"
                                    enabled: modelData.url && modelData.url.length > 0
                                    onClicked: Qt.openUrlExternally(modelData.url)
                                }
                            }
                        }
                    }

                    Kirigami.InlineMessage {
                        Layout.fillWidth: true
                        visible: !page.backend.installRunning
                                 && page.backend.installMissing.length > 0
                                 && page.backend.install.can_install === false
                        type: Kirigami.MessageType.Warning
                        text: page.backend.install.reason || "Automatic installation is not available."
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                        Controls.Button {
                            text: page.backend.installRunning
                                ? "Installing…"
                                : "Install available packages"
                            icon.name: "system-software-update"
                            enabled: page.backend.install.can_install === true
                                     && page.backend.installMissing.length > 0
                                     && !page.backend.installRunning
                            onClicked: page.backend.installDependencies()
                        }
                        Item { Layout.fillWidth: true }
                    }

                    Controls.Label {
                        visible: page.backend.installMissing.length > 0
                                 && page.backend.install.command
                                 && page.backend.install.command.length > 0
                        text: "Administrator access is requested through Polkit. Command: "
                              + page.backend.install.command
                        color: Kirigami.Theme.disabledTextColor
                        font.pointSize: Kirigami.Theme.smallFont.pointSize
                        wrapMode: Text.WordWrap
                        Layout.fillWidth: true
                    }

                    Kirigami.InlineMessage {
                        Layout.fillWidth: true
                        visible: page.backend.install.status
                                 && page.backend.install.status.error
                                 && page.backend.install.status.error.length > 0
                        type: Kirigami.MessageType.Error
                        text: page.backend.install.status
                            ? page.backend.install.status.error : ""
                    }

                    Controls.ScrollView {
                        Layout.fillWidth: true
                        Layout.preferredHeight: 180
                        visible: page.backend.install.status
                                 && page.backend.install.status.log
                                 && page.backend.install.status.log.length > 0
                        Controls.TextArea {
                            text: page.backend.install.status
                                ? page.backend.install.status.log : ""
                            readOnly: true
                            font.family: "monospace"
                            font.pointSize: Kirigami.Theme.smallFont.pointSize
                            wrapMode: TextEdit.WrapAnywhere
                            onTextChanged: cursorPosition = length
                        }
                    }
                }
            }

            Kirigami.InlineMessage {
                Layout.fillWidth: true
                visible: page.backend.diagnostics.message !== undefined
                         && page.backend.diagnostics.message !== null
                         && String(page.backend.diagnostics.message).length > 0
                type: Kirigami.MessageType.Error
                text: page.backend.diagnostics.message || ""
            }

            Kirigami.Card {
                Layout.fillWidth: true
                visible: page.backend.diagnostics.checks !== undefined
                contentItem: ColumnLayout {
                    spacing: Kirigami.Units.smallSpacing

                    RowLayout {
                        Layout.fillWidth: true
                        KirigamiPrimitives.Icon {
                            source: (page.backend.diagnostics.error || 0) > 0
                                ? "data-error" : "dialog-ok"
                            color: (page.backend.diagnostics.error || 0) > 0
                                ? Kirigami.Theme.negativeTextColor
                                : Kirigami.Theme.positiveTextColor
                            Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                            Layout.preferredHeight: Kirigami.Units.iconSizes.medium
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            Controls.Label {
                                text: (page.backend.diagnostics.error || 0) > 0
                                    ? "Some checks need attention"
                                    : "Ready to stream"
                                font.weight: Font.DemiBold
                            }
                            Controls.Label {
                                text: (page.backend.diagnostics.ok || 0) + " ok  ·  "
                                      + (page.backend.diagnostics.warn || 0) + " warnings  ·  "
                                      + (page.backend.diagnostics.error || 0) + " errors"
                                color: Kirigami.Theme.disabledTextColor
                                font.pointSize: Kirigami.Theme.smallFont.pointSize
                            }
                        }
                    }

                    Controls.Label {
                        Layout.fillWidth: true
                        wrapMode: Text.WordWrap
                        text: page.backend.diagnostics.desktop + " / "
                              + page.backend.diagnostics.session_type
                              + "  ·  KDE Fusion " + page.backend.diagnostics.app_version
                        color: Kirigami.Theme.disabledTextColor
                    }
                }
            }

            Repeater {
                model: page.backend.diagnostics.checks || []
                delegate: Kirigami.Card {
                    required property var modelData
                    Layout.fillWidth: true
                    contentItem: ColumnLayout {
                        spacing: Kirigami.Units.smallSpacing

                        RowLayout {
                            Layout.fillWidth: true
                            KirigamiPrimitives.Icon {
                                source: modelData.status === "ok" ? "dialog-ok"
                                    : modelData.status === "error" ? "data-error"
                                    : modelData.status === "warn" ? "data-warning"
                                    : "dialog-information"
                                color: page.statusColor(modelData.status)
                                Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                                Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                            }
                            Controls.Label {
                                text: modelData.label
                                font.weight: Font.DemiBold
                                Layout.fillWidth: true
                                elide: Text.ElideRight
                            }
                            Controls.Label {
                                text: page.statusLabel(modelData.status)
                                color: page.statusColor(modelData.status)
                                font.pointSize: Kirigami.Theme.smallFont.pointSize
                            }
                        }

                        Controls.Label {
                            text: modelData.detail
                            wrapMode: Text.WordWrap
                            Layout.fillWidth: true
                            color: Kirigami.Theme.disabledTextColor
                        }

                        Controls.Label {
                            visible: modelData.fix !== undefined && modelData.fix.length > 0
                            text: "Fix: " + modelData.fix
                            wrapMode: Text.WordWrap
                            Layout.fillWidth: true
                            color: Kirigami.Theme.neutralTextColor
                        }
                    }
                }
            }

            Kirigami.Heading { level: 2; text: "Logs" }

            Kirigami.Card {
                Layout.fillWidth: true
                contentItem: ColumnLayout {
                    spacing: Kirigami.Units.smallSpacing
                    Controls.Label {
                        text: "Daemon log:"
                        font.weight: Font.DemiBold
                    }
                    Controls.Label {
                        text: "journalctl --user -u kdefusiond -n 200 --no-pager"
                        font.family: "monospace"
                        wrapMode: Text.WrapAnywhere
                        Layout.fillWidth: true
                    }
                    Controls.Label {
                        text: "Set the daemon log level with: systemctl --user edit kdefusiond " +
                              "and add --log-level debug to ExecStart."
                        color: Kirigami.Theme.disabledTextColor
                        font.pointSize: Kirigami.Theme.smallFont.pointSize
                        wrapMode: Text.WordWrap
                        Layout.fillWidth: true
                    }
                }
            }

            Item { Layout.preferredHeight: Kirigami.Units.largeSpacing }
        }
    }
}
