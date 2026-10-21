import QtQuick
import QtQuick.Controls as QQC
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

// Layoutless bar widget: a keyboard icon (dimmed while auto-correction is
// paused) and a panel with the auto toggle and the learned words. All state
// lives in the layoutless daemon; this widget only talks to it through the
// `layoutless` CLI: `subscribe` for live state, `ctl` for edits. Commands go
// to `ctl` on stdin, never as arguments: words in a command line are visible
// to every local user through `ps`.
Panel {
  id: root
  moduleName: "kladafox.layoutless"
  ipcTarget: "kladafox.layoutless"

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  // From the daemon. `daemonUp` is false while `layoutless subscribe` isn't
  // connected (daemon stopped or restarting).
  property bool daemonUp: false
  property bool auto: false
  property int ruleCount: 0
  property string flipKey: "ALT + SPACE"
  // The focused window's class (the panel itself never takes it) and the
  // apps where auto-correction is off.
  property string app: ""
  property bool appExcluded: false
  property var excludeApps: []
  property int learnThreshold: 2
  // [{keys, en, ru, lang, created_at}], newest first.
  property var rules: []
  // The add field doubles as a filter: the list shows the rules where either
  // spelling contains what's typed (so `ghbdtn` finds `привет` too).
  readonly property string filter: addField.text.trim().toLowerCase()
  readonly property var shownRules: filter === ""
    ? rules
    : rules.filter(function(r) {
        return r.en.toLowerCase().indexOf(filter) !== -1 || r.ru.toLowerCase().indexOf(filter) !== -1
      })
  onFilterChanged: cursorIndex = -1
  property string lastError: ""

  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family
  readonly property color dim: Qt.darker(foreground, 1.55)

  // Row cursor, per the CursorSurface contract: hover moves it.
  property int cursorIndex: -1

  // The word as it belongs, and how it gets typed by mistake.
  function wordOf(rule) { return rule.lang === "en" ? rule.en : rule.ru }
  function otherOf(rule) { return rule.lang === "en" ? rule.ru : rule.en }

  function refresh() {
    if (!listProc.running) listProc.running = true
  }

  function run(cmd) {
    if (actionProc.running) return
    actionProc.pendingCommand = cmd
    actionProc.running = true
  }

  function addWord() {
    var word = addField.text.trim()
    if (word === "") return
    run("rules add " + word)
    addField.text = ""
  }

  onOpenedChanged: {
    if (opened) {
      refresh()
      Qt.callLater(function() { keyCatcher.forceActiveFocus() })
    }
  }

  Process {
    id: watchProc
    command: ["setpriv", "--pdeathsig", "TERM", "layoutless", "subscribe"]
    onRunningChanged: if (!running) root.daemonUp = false
    onExited: watchRestart.restart()
    stdout: SplitParser {
      onRead: function(line) {
        var ev
        try { ev = JSON.parse(line) } catch (e) { return }
        root.daemonUp = true
        if (ev.event === "state") {
          root.auto = ev.auto === true
          if (ev.flip_key) root.flipKey = ev.flip_key
          if (ev.learn_threshold) root.learnThreshold = ev.learn_threshold
          root.app = ev.app || ""
          root.appExcluded = ev.app_excluded === true
          root.excludeApps = ev.exclude_apps || []
          if (ev.rules !== root.ruleCount && root.opened) root.refresh()
          root.ruleCount = ev.rules || 0
        }
      }
    }
  }

  Timer {
    id: watchRestart
    interval: 2000
    onTriggered: if (!watchProc.running) watchProc.running = true
  }

  Component.onCompleted: watchProc.running = true

  Process {
    id: listProc
    command: ["layoutless", "rules", "list", "--json"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var list
        try { list = JSON.parse(text || "{}").rules || [] } catch (e) { return }
        list.sort(function(a, b) { return b.created_at - a.created_at })
        root.rules = list
      }
    }
  }

  Process {
    id: actionProc
    property string pendingCommand: ""
    command: ["layoutless", "ctl"]
    stdinEnabled: true
    onStarted: write(pendingCommand + "\n")
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var out = String(text || "").trim()
        root.lastError = out.indexOf("error") === 0 ? out.replace(/^error:\s*/, "") : ""
      }
    }
    onExited: root.refresh()
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: "󰌌"
    dimmed: !root.daemonUp || !root.auto
    tooltipText: !root.daemonUp
      ? "Layoutless: daemon not running"
      : "Layoutless: auto-correction " + (root.auto ? "on" : "paused") + "\nRight click to " + (root.auto ? "pause" : "resume")
    onPressed: function(b) {
      if (b === Qt.RightButton) root.run("auto toggle")
      else root.toggle()
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(340))
    contentHeight: panel.fittedContentHeight(column.implicitHeight, Style.space(900))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      blocked: addField.activeFocus
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }

      Column {
        id: column
        width: parent.width
        spacing: Style.space(10)

        Text {
          visible: !root.daemonUp
          width: parent.width
          wrapMode: Text.Wrap
          textFormat: Text.PlainText
          text: "The layoutless daemon is not running.\nStart it with: systemctl --user start layoutless"
          color: root.foreground
          font.family: root.fontFamily
          font.pixelSize: Style.font.body
        }

        Toggle {
          visible: root.daemonUp
          width: parent.width
          label: "Auto-correction"
          description: "Retype words typed in the wrong layout"
          checked: root.auto
          foreground: root.foreground
          fontFamily: root.fontFamily
          onClicked: root.run("auto toggle")
        }

        Toggle {
          visible: root.daemonUp && root.app !== ""
          width: parent.width
          label: "Don't correct in " + root.app
          description: root.flipKey + " still works there"
          checked: root.appExcluded
          foreground: root.foreground
          fontFamily: root.fontFamily
          onClicked: root.run("exclude " + (root.appExcluded ? "rm " : "add ") + root.app)
        }

        PanelSeparator { visible: root.daemonUp; foreground: root.foreground }

        PanelSectionHeader {
          visible: root.daemonUp
          text: root.filter === ""
            ? "LEARNED WORDS (" + root.rules.length + ")"
            : "LEARNED WORDS (" + root.shownRules.length + " OF " + root.rules.length + ")"
          foreground: root.foreground
          fontFamily: root.fontFamily
        }

        Row {
          visible: root.daemonUp
          width: parent.width
          spacing: Style.space(8)

          TextField {
            id: addField
            width: parent.width - addButton.width - parent.spacing
            placeholderText: "Find or add a word: next, привет"
            foreground: root.foreground
            font.family: root.fontFamily
            Keys.onPressed: function(event) {
              if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                root.addWord()
                event.accepted = true
              } else if (event.key === Qt.Key_Escape) {
                addField.text = ""
                keyCatcher.forceActiveFocus()
                event.accepted = true
              }
            }
          }

          Button {
            id: addButton
            text: "Add"
            iconText: "󰐕"
            bordered: true
            foreground: root.foreground
            fontFamily: root.fontFamily
            onClicked: root.addWord()
          }
        }

        Text {
          visible: root.lastError !== ""
          width: parent.width
          wrapMode: Text.Wrap
          textFormat: Text.PlainText
          text: root.lastError
          color: root.bar ? root.bar.urgent : Color.urgent
          font.family: root.fontFamily
          font.pixelSize: Style.font.bodySmall
        }

        Text {
          visible: root.daemonUp && root.rules.length === 0
          width: parent.width
          wrapMode: Text.Wrap
          textFormat: Text.PlainText
          text: "No learned words yet. Flip the same word with " + root.flipKey
            + (root.learnThreshold === 1 ? " once" : " " + root.learnThreshold + " times")
            + " and it's remembered."
          color: root.dim
          font.family: root.fontFamily
          font.pixelSize: Style.font.bodySmall
        }

        Text {
          visible: root.daemonUp && root.rules.length > 0 && root.shownRules.length === 0
          width: parent.width
          wrapMode: Text.Wrap
          textFormat: Text.PlainText
          text: "Nothing matches. Press Enter to add it."
          color: root.dim
          font.family: root.fontFamily
          font.pixelSize: Style.font.bodySmall
        }

        // A ListView that only scrolls when the rows don't fit, like the
        // Bluetooth panel: a Flickable here kept nudging the content under a
        // still pointer, so the row buttons flickered between hovered and not.
        ListView {
          id: rulesView
          visible: root.daemonUp && root.shownRules.length > 0
          width: parent.width
          height: Math.min(contentHeight, Style.space(320))
          clip: true
          boundsBehavior: Flickable.StopAtBounds
          interactive: contentHeight > height
          model: root.shownRules

          // Always shown while the list scrolls, so it's clear there's more
          // (AsNeeded only shows it during a scroll).
          QQC.ScrollBar.vertical: QQC.ScrollBar {
            policy: rulesView.interactive ? QQC.ScrollBar.AlwaysOn : QQC.ScrollBar.AlwaysOff
          }

          delegate: CursorSurface {
            id: row
            required property int index
            required property var modelData
            // Leave room for the scrollbar when the list scrolls.
            width: ListView.view.width - (rulesView.interactive ? Style.space(10) : 0)
            height: Style.space(34)
            foreground: root.foreground
            hasCursor: root.cursorIndex === row.index

            // The row counts as hovered while the pointer is on it or on one
            // of its buttons: the row's MouseArea reports leaving as soon as
            // the pointer moves onto a button.
            property int hotButtons: 0
            readonly property bool hot: rowMouse.containsMouse || hotButtons > 0
            onHotChanged: {
              if (hot) root.cursorIndex = row.index
              else if (root.cursorIndex === row.index) root.cursorIndex = -1
            }

            MouseArea {
              id: rowMouse
              anchors.fill: parent
              hoverEnabled: true
            }

            Text {
              anchors.left: parent.left
              anchors.leftMargin: Style.space(8)
              anchors.verticalCenter: parent.verticalCenter
              textFormat: Text.PlainText
              text: root.wordOf(row.modelData)
              color: root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.body
            }

            Text {
              anchors.left: parent.left
              anchors.leftMargin: Style.space(130)
              anchors.verticalCenter: parent.verticalCenter
              textFormat: Text.PlainText
              text: "← " + root.otherOf(row.modelData)
              color: root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.bodySmall
            }

            Row {
              anchors.right: parent.right
              anchors.rightMargin: Style.space(6)
              anchors.verticalCenter: parent.verticalCenter
              spacing: Style.space(4)
              // Always there, faint until the row is hovered.
              opacity: row.hasCursor ? 1 : 0.35

              PanelActionButton {
                iconText: "󰓡"
                foreground: root.dim
                fontFamily: root.fontFamily
                onHovered: function(on) { row.hotButtons += on ? 1 : -1 }
                onClicked: root.run("rules set " + row.modelData.keys + " " + (row.modelData.lang === "en" ? "ru" : "en"))
              }

              PanelActionButton {
                iconText: "󰆴"
                foreground: root.dim
                hoverColor: root.bar ? root.bar.urgent : Color.urgent
                fontFamily: root.fontFamily
                onHovered: function(on) { row.hotButtons += on ? 1 : -1 }
                onClicked: root.run("rules rm " + root.wordOf(row.modelData))
              }
            }
          }
        }

        Text {
          visible: root.daemonUp && root.rules.length > 0
          width: parent.width
          wrapMode: Text.Wrap
          textFormat: Text.PlainText
          text: "󰓡 keep the other spelling instead   󰆴 forget"
          color: root.dim
          font.family: root.fontFamily
          font.pixelSize: Style.font.bodySmall
        }

        PanelSeparator { visible: root.daemonUp && root.excludeApps.length > 0; foreground: root.foreground }

        PanelSectionHeader {
          visible: root.daemonUp && root.excludeApps.length > 0
          text: "NOT CORRECTED IN (" + root.excludeApps.length + ")"
          foreground: root.foreground
          fontFamily: root.fontFamily
        }

        Column {
          visible: root.daemonUp && root.excludeApps.length > 0
          width: parent.width

          Repeater {
            model: root.excludeApps
            delegate: Item {
              id: appRow
              required property var modelData
              width: parent.width
              height: Style.space(28)

              Text {
                anchors.left: parent.left
                anchors.leftMargin: Style.space(8)
                anchors.right: removeApp.left
                anchors.verticalCenter: parent.verticalCenter
                elide: Text.ElideRight
                textFormat: Text.PlainText
                text: appRow.modelData
                color: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.bodySmall
              }

              PanelActionButton {
                id: removeApp
                anchors.right: parent.right
                anchors.rightMargin: Style.space(6)
                anchors.verticalCenter: parent.verticalCenter
                iconText: "󰆴"
                foreground: root.dim
                hoverColor: root.bar ? root.bar.urgent : Color.urgent
                fontFamily: root.fontFamily
                onClicked: root.run("exclude rm " + appRow.modelData)
              }
            }
          }
        }
      }
    }
  }
}
