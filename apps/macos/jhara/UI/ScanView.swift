// ScanView.swift
// Shown during .idle and .scanning phases.

import SwiftUI

struct ScanView: View {
    @Environment(AppState.self) var appState

    // Paths to scan — defaults to home directory.
    @State private var customPath: String = FileManager.default
        .homeDirectoryForCurrentUser.path

    var body: some View {
        VStack(spacing: 0) {
            header
            Spacer()
            centerContent
            Spacer()
            footer
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 20)
    }

    // MARK: - Header

    private var header: some View {
        HStack {
            Image(systemName: "sparkles")
                .font(.title2)
                .foregroundStyle(.purple)
            Text("Jhara")
                .font(.title2.bold())
            Spacer()
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Jhara disk cleaner")
    }

    // MARK: - Center content

    @ViewBuilder
    private var centerContent: some View {
        if appState.isScanning {
            scanningContent
        } else {
            idleContent
        }
    }

    private var idleContent: some View {
        VStack(spacing: 20) {
            Image(systemName: "internaldrive")
                .font(.system(size: 52))
                .foregroundStyle(.purple.gradient)
                .accessibilityHidden(true)

            Text("Find Disk Clutter")
                .font(.title3.bold())

            Text("Jhara scans your project directories and identifies safe-to-remove build artefacts, caches, and stale projects.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)

            // Path picker
            HStack {
                TextField("Scan path", text: $customPath)
                    .textFieldStyle(.roundedBorder)
                    .font(.caption)
                    .accessibilityLabel("Directory to scan")

                Button {
                    choosePath()
                } label: {
                    Image(systemName: "folder")
                }
                .accessibilityLabel("Choose directory")
            }

            Button {
                appState.startScan(roots: [customPath])
            } label: {
                Label("Start Scan", systemImage: "magnifyingglass")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .tint(.purple)
            .disabled(customPath.isEmpty)
            .accessibilityHint("Scans the selected directory for reclaimable disk space")
        }
    }

    private var scanningContent: some View {
        VStack(spacing: 24) {
            ProgressRing(progress: scanProgress)
                .frame(width: 120, height: 120)
                .accessibilityValue("\(Int(scanProgress * 100)) percent complete")
                .accessibilityLabel("Scan progress")

            VStack(spacing: 6) {
                Text("Scanning…")
                    .font(.title3.bold())
                Text(customPath)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }

            Button("Cancel") {
                appState.cancelScan()
            }
            .buttonStyle(.bordered)
            .tint(.red)
            .accessibilityHint("Stops the current scan")
        }
    }

    // MARK: - Footer

    private var footer: some View {
        HStack {
            if appState.orphanDetectionAvailable {
                Label("Apple Silicon", systemImage: "cpu")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Apple Silicon orphan detection available")
            }
            Spacer()
            Text("v1.0")
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
    }

    // MARK: - Helpers

    private var scanProgress: Double {
        if case .scanning(let p) = appState.phase { return p }
        return 0
    }

    private func choosePath() {
        let panel = NSOpenPanel()
        panel.canChooseFiles       = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "Scan"
        if panel.runModal() == .OK, let url = panel.url {
            customPath = url.path
        }
    }
}

// MARK: - ProgressRing

/// Animated circular progress indicator.
struct ProgressRing: View {
    let progress: Double

    @State private var animatedProgress: Double = 0
    @State private var rotating = false

    var body: some View {
        ZStack {
            // Track
            Circle()
                .stroke(Color.purple.opacity(0.15), lineWidth: 8)

            // Fill
            Circle()
                .trim(from: 0, to: animatedProgress)
                .stroke(
                    AngularGradient(
                        gradient: Gradient(colors: [.purple, .pink]),
                        center:   .center
                    ),
                    style: StrokeStyle(lineWidth: 8, lineCap: .round)
                )
                .rotationEffect(.degrees(-90))
                .animation(.easeInOut(duration: 0.4), value: animatedProgress)

            // Spinning dots when indeterminate (progress < 0.05)
            if animatedProgress < 0.05 {
                Circle()
                    .fill(.purple)
                    .frame(width: 6, height: 6)
                    .offset(y: -44)
                    .rotationEffect(.degrees(rotating ? 360 : 0))
                    .animation(
                        .linear(duration: 1).repeatForever(autoreverses: false),
                        value: rotating
                    )
            }

            Text("\(Int(animatedProgress * 100))%")
                .font(.system(size: 20, weight: .bold, design: .rounded))
                .foregroundStyle(.primary)
                .contentTransition(.numericText())
        }
        .onAppear {
            animatedProgress = progress
            rotating         = true
        }
        .onChange(of: progress) { _, new in
            animatedProgress = new
        }
    }
}
