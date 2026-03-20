// JharaApp.swift
// jhara — macOS Menu Bar Application
//
// No Dock icon. NSStatusItem drives presence.
// NSPanel main window floats above desktop, dismisses on focus loss.

import SwiftUI
import AppKit

@main
struct JharaApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var appDelegate

    var body: some Scene {
        // We manage our own window via AppDelegate + NSPanel.
        // This empty Settings scene keeps SwiftUI happy without a main window.
        Settings { EmptyView() }
    }
}

// MARK: - AppDelegate

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {

    private var statusItem:  NSStatusItem?
    private var panel:       NSPanel?
    private var popover:     NSPopover?

    // Shared app state — injected into the SwiftUI tree.
    let appState = AppState()

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Suppress Dock icon at runtime (LSUIElement=YES in Info.plist is
        // the static approach; this handles dynamic launch).
        NSApp.setActivationPolicy(.accessory)

        setupStatusItem()
    }

    // MARK: - Status item

    private func setupStatusItem() {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)

        if let button = statusItem?.button {
            button.image = NSImage(
                systemSymbolName: "sparkles",
                accessibilityDescription: "Jhara"
            )
            button.action = #selector(togglePanel)
            button.target = self
        }
    }

    @objc private func togglePanel() {
        if let panel, panel.isVisible {
            panel.orderOut(nil)
            return
        }
        showPanel()
    }

    // MARK: - NSPanel

    private func showPanel() {
        if panel == nil {
            let contentView = ContentView()
                .environment(appState)

            let hosting = NSHostingController(rootView: contentView)
            hosting.view.frame = NSRect(x: 0, y: 0, width: 420, height: 580)

            let p = NSPanel(
                contentRect: hosting.view.frame,
                styleMask:   [.titled, .closable, .fullSizeContentView, .nonactivatingPanel],
                backing:     .buffered,
                defer:       false
            )
            p.titlebarAppearsTransparent = true
            p.titleVisibility           = .hidden
            p.isFloatingPanel           = true
            p.becomesKeyOnlyIfNeeded    = true
            p.contentViewController     = hosting
            p.isReleasedWhenClosed      = false

            // Dismiss on focus loss.
            NotificationCenter.default.addObserver(
                self,
                selector: #selector(panelDidResignKey),
                name:     NSWindow.didResignKeyNotification,
                object:   p
            )

            panel = p
        }

        // Position below the status item button.
        if let button = statusItem?.button,
           let buttonWindow = button.window {
            let buttonRect   = buttonWindow.convertToScreen(button.frame)
            let panelSize    = panel!.frame.size
            let origin = NSPoint(
                x: buttonRect.midX - panelSize.width / 2,
                y: buttonRect.minY - panelSize.height - 4
            )
            panel!.setFrameOrigin(origin)
        }

        panel!.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    @objc private func panelDidResignKey() {
        panel?.orderOut(nil)
    }
}
