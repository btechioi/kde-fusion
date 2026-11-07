import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

import "components" as Components
import "pages" as Pages

Kirigami.ApplicationWindow {
    id: root

    visible: true
    width: 1000
    height: 760
    minimumWidth: 760
    minimumHeight: 560
    title: "KDE Fusion"
    desktopFileName: "kdefusion"

    property string currentPage: "devices"
    property bool smokeTest: Qt.application.arguments.indexOf("--smoke-test") >= 0
    property bool quitting: false
    readonly property string screenshotPath: {
        for (var i = 0; i < Qt.application.arguments.length; i++) {
            var argument = Qt.application.arguments[i];
            if (argument.indexOf("--screenshot=") === 0) {
                return argument.substring("--screenshot=".length);
            }
        }
        return "";
    }
    readonly property string initialPage: {
        for (var i = 0; i < Qt.application.arguments.length; i++) {
            var argument = Qt.application.arguments[i];
            if (argument.indexOf("--page=") === 0) {
                return argument.substring("--page=".length);
            }
        }
        return "";
    }
    readonly property string apiBaseOverride: {
        for (var i = 0; i < Qt.application.arguments.length; i++) {
            var argument = Qt.application.arguments[i];
            if (argument.indexOf("--api-base=") === 0) {
                return argument.substring("--api-base=".length);
            }
        }
        return "";
    }

    Components.Backend {
        id: backend
    }

    function navigate(page) {
        if (page === root.currentPage) {
            if (root.globalDrawer) {
                root.globalDrawer.close();
            }
            return;
        }
        var urls = {
            "devices": "pages/DevicesPage.qml",
            "session": "pages/SessionPage.qml",
            "settings": "pages/SettingsPage.qml",
            "diagnostics": "pages/DiagnosticsPage.qml"
        };
        if (!urls[page]) {
            return;
        }
        var properties = {
            backend: backend,
            navigate: root.navigate
        };
        if (root.pageStack && root.pageStack.depth > 0) {
            root.pageStack.replace(Qt.resolvedUrl(urls[page]), properties);
        } else {
            root.pageStack.push(Qt.resolvedUrl(urls[page]), properties);
        }
        root.currentPage = page;
        if (root.globalDrawer) {
            root.globalDrawer.close();
        }
    }

    function navigateToSession(peerId, mode) {
        var properties = {
            backend: backend,
            navigate: root.navigate,
            preselectPeerId: peerId || "",
            preselectMode: mode || "mirror"
        };
        root.pageStack.replace(Qt.resolvedUrl("pages/SessionPage.qml"), properties);
        root.currentPage = "session";
    }

    function quitApp() {
        root.quitting = true;
        Qt.quit();
    }

    pageStack.initialPage: Pages.DevicesPage {
        backend: backend
        navigate: root.navigate
        onNavigateToSession: function (peerId, mode) {
            root.navigateToSession(peerId, mode);
        }
    }

    globalDrawer: Kirigami.GlobalDrawer {
        id: drawer
        title: "KDE Fusion"
        titleIcon: "kdefusion"
        isMenu: false
        modal: false
        collapsed: false
        collapsible: true
        actions: [
            Kirigami.Action {
                text: "Devices"
                icon.name: "network-wired"
                checkable: true
                checked: root.currentPage === "devices"
                onTriggered: root.navigate("devices")
            },
            Kirigami.Action {
                text: "Session"
                icon.name: "media-playback-start"
                checkable: true
                checked: root.currentPage === "session"
                onTriggered: root.navigate("session")
            },
            Kirigami.Action {
                text: "Settings"
                icon.name: "settings-configure"
                checkable: true
                checked: root.currentPage === "settings"
                onTriggered: root.navigate("settings")
            },
            Kirigami.Action {
                text: "Diagnostics"
                icon.name: "utilities-system-monitor"
                checkable: true
                checked: root.currentPage === "diagnostics"
                onTriggered: root.navigate("diagnostics")
            },
            Kirigami.Action {
                text: "About"
                icon.name: "help-about"
                onTriggered: aboutDialog.open()
            },
            Kirigami.Action {
                text: "Quit"
                icon.name: "application-exit"
                onTriggered: root.quitApp()
            }
        ]
    }

    footer: Controls.ToolBar {
        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: Kirigami.Units.largeSpacing
            anchors.rightMargin: Kirigami.Units.largeSpacing
            spacing: Kirigami.Units.smallSpacing

            Rectangle {
                width: Kirigami.Units.smallSpacing
                height: Kirigami.Units.smallSpacing
                radius: width / 2
                color: backend.connected
                    ? Kirigami.Theme.positiveTextColor
                    : Kirigami.Theme.negativeTextColor
            }

            Controls.Label {
                text: backend.connected
                    ? "Daemon connected"
                    : "Daemon offline"
                font.pointSize: Kirigami.Theme.smallFont.pointSize
                color: Kirigami.Theme.disabledTextColor
            }

            Controls.Label {
                visible: backend.active
                text: "  ·  " + backend.phaseLabel(backend.session.phase)
                      + (backend.session.peer_name ? " with " + backend.session.peer_name : "")
                font.pointSize: Kirigami.Theme.smallFont.pointSize
            }

            Item { Layout.fillWidth: true }

            Controls.Label {
                text: backend.deviceName
                font.pointSize: Kirigami.Theme.smallFont.pointSize
                color: Kirigami.Theme.disabledTextColor
            }
        }
    }

    Connections {
        target: backend
        function onLastErrorChanged() {
            if (backend.lastError.length > 0) {
                root.showPassiveNotification(backend.lastError, "long");
            }
        }
        function onOpenRequested(page) {
            root.show();
            root.raise();
            root.requestActivate();
            if (page && page.length > 0) {
                root.navigate(page);
            }
        }
    }

    onClosing: function (close) {
        if (root.quitting || root.smokeTest || root.screenshotPath.length > 0) {
            return;
        }
        var closeToTray = backend.config && backend.config.ui
            ? backend.config.ui.close_to_tray : false;
        if (closeToTray) {
            close.accepted = false;
            root.hide();
        }
    }

    Controls.Dialog {
        id: aboutDialog
        modal: true
        title: "About KDE Fusion"
        standardButtons: Controls.Dialog.Close
        anchors.centerIn: parent
        contentItem: ColumnLayout {
            spacing: Kirigami.Units.smallSpacing
            Controls.Label {
                text: "KDE Fusion"
                font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.4
                font.weight: Font.Bold
            }
            Controls.Label {
                text: "Mirror or extend a KDE Plasma Wayland desktop using Sunshine and Moonlight."
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
            Controls.Label {
                text: "Version " + (backend.status.version || "0.1.0")
                color: Kirigami.Theme.disabledTextColor
            }
            Controls.Label {
                text: "KDE-native frontend. LAN-only control channel with explicit pairing."
                color: Kirigami.Theme.disabledTextColor
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
        }
    }

    Timer {
        id: smokeTimer
        interval: 1800
        repeat: false
        onTriggered: Qt.quit()
    }

    Timer {
        id: screenshotTimer
        interval: 1600
        repeat: false
        onTriggered: {
            root.requestActivate();
            var item = root.pageStack && root.pageStack.currentItem
                ? root.pageStack.currentItem : root.contentItem;
            item.grabToImage(function (result) {
                result.saveToFile(root.screenshotPath);
                Qt.quit();
            });
        }
    }

    Component.onCompleted: {
        if (root.apiBaseOverride.length > 0) {
            backend.apiBase = root.apiBaseOverride;
        }
        backend.refreshAll();
        if (root.initialPage.length > 0) {
            root.navigate(root.initialPage);
        }
        if (root.screenshotPath.length > 0) {
            screenshotTimer.start();
        } else if (root.smokeTest) {
            smokeTimer.start();
        }
    }
}
