// ContentView.swift
// Root view — switches between ScanView and ResultsView based on AppState.phase.

import SwiftUI

struct ContentView: View {
    @Environment(AppState.self) var appState

    var body: some View {
        ZStack {
            // Background material
            Rectangle()
                .fill(.ultraThinMaterial)
                .ignoresSafeArea()

            switch appState.phase {
            case .idle:
                ScanView()
                    .transition(.opacity.combined(with: .scale(scale: 0.96)))

            case .scanning:
                ScanView()
                    .transition(.opacity)

            case .done:
                ResultsView()
                    .transition(.opacity.combined(with: .move(edge: .trailing)))

            case .failed(let msg):
                ErrorView(message: msg)
                    .transition(.opacity)
            }
        }
        .frame(width: 420, height: 580)
        .animation(.easeInOut(duration: 0.25), value: phaseKey)
        // Deletion confirmation sheets — presented over any view.
        .sheet(isPresented: isDeletionPresented) {
            DeletionConfirmationView()
                .environment(appState)
        }
    }

    // A simple Equatable key so the ZStack animation fires on phase changes.
    private var phaseKey: String {
        switch appState.phase {
        case .idle:       return "idle"
        case .scanning:   return "scanning"
        case .done:       return "done"
        case .failed:     return "failed"
        }
    }

    private var isDeletionPresented: Binding<Bool> {
        Binding(
            get: {
                if case .none = appState.deletionFlow { return false }
                return true
            },
            set: { if !$0 { appState.deletionFlow = .none } }
        )
    }
}

// MARK: - ErrorView

struct ErrorView: View {
    let message: String
    @Environment(AppState.self) var appState

    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 40))
                .foregroundStyle(.red)
            Text("Scan Failed")
                .font(.headline)
            Text(message)
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Button("Try Again") {
                appState.phase = .idle
            }
            .buttonStyle(.borderedProminent)
        }
        .padding(32)
    }
}
