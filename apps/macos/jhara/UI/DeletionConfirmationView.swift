// DeletionConfirmationView.swift
// Deletion confirmation flows for all three safety tiers.
//
// Safe    → one-click, summary sheet
// Caution → explicit checkbox per category with explanation
// Risky   → per-item dialog, no batch removal

import SwiftUI

struct DeletionConfirmationView: View {
    @Environment(AppState.self) var appState
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Group {
            switch appState.deletionFlow {
            case .confirmSafe(let artifacts, let projectID):
                SafeDeletionSheet(artifacts: artifacts, projectID: projectID)
            case .confirmCaution(let artifacts, let projectID):
                CautionDeletionSheet(artifacts: artifacts, projectID: projectID)
            case .confirmRisky(let artifact):
                RiskyDeletionSheet(artifact: artifact)
            case .none:
                EmptyView()
            }
        }
        .environment(appState)
    }
}

// MARK: - Safe deletion

struct SafeDeletionSheet: View {
    let artifacts:  [Artifact]
    let projectID:  UUID
    @Environment(AppState.self) var appState
    @Environment(\.dismiss) private var dismiss
    @State private var deleting = false

    var totalSize: Int64 { artifacts.reduce(0) { $0 + $1.size } }

    var body: some View {
        VStack(spacing: 20) {
            // Header
            HStack(spacing: 12) {
                Image(systemName: "trash.fill")
                    .font(.title2)
                    .foregroundStyle(.green)
                VStack(alignment: .leading) {
                    Text("Remove Safe Items")
                        .font(.headline)
                    Text("These items are safe to delete with no side effects.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            Divider()

            // Summary
            VStack(alignment: .leading, spacing: 6) {
                summaryRow(label: "Items to remove", value: "\(artifacts.count)")
                summaryRow(
                    label: "Space freed",
                    value: ByteCountFormatter.string(fromByteCount: totalSize, countStyle: .file)
                )
            }
            .padding(12)
            .background(.quaternary, in: RoundedRectangle(cornerRadius: 8))

            // Artifact list (compact)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 4) {
                    ForEach(artifacts) { artifact in
                        HStack {
                            Text(artifact.name)
                                .font(.caption)
                                .lineLimit(1)
                            Spacer()
                            Text(ByteCountFormatter.string(
                                fromByteCount: artifact.size, countStyle: .file
                            ))
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                        }
                        .padding(.horizontal, 4)
                    }
                }
            }
            .frame(maxHeight: 140)
            .background(.quinary, in: RoundedRectangle(cornerRadius: 6))

            Spacer()

            // Actions
            HStack {
                Button("Cancel") {
                    appState.deletionFlow = .none
                    dismiss()
                }
                .keyboardShortcut(.cancelAction)
                .accessibilityLabel("Cancel deletion")

                Spacer()

                Button {
                    performDeletion()
                } label: {
                    if deleting {
                        ProgressView().scaleEffect(0.7)
                    } else {
                        Label("Delete \(artifacts.count) Items", systemImage: "trash")
                    }
                }
                .buttonStyle(.borderedProminent)
                .tint(.green)
                .disabled(deleting)
                .keyboardShortcut(.defaultAction)
                .accessibilityHint("Permanently deletes the listed safe items")
            }
        }
        .padding(24)
        .frame(width: 380)
    }

    private func summaryRow(label: String, value: String) -> some View {
        HStack {
            Text(label).font(.caption).foregroundStyle(.secondary)
            Spacer()
            Text(value).font(.caption.bold())
        }
    }

    private func performDeletion() {
        deleting = true
        Task {
            // TODO(phase-7): call Rust deletion API.
            // Simulate async deletion for now.
            try? await Task.sleep(for: .seconds(0.6))
            await MainActor.run {
                // Remove from app state
                if let idx = appState.projects.firstIndex(where: { $0.id == projectID }) {
                    let ids = Set(artifacts.map(\.id))
                    appState.projects[idx].artifacts.removeAll { ids.contains($0.id) }
                }
                appState.deletionFlow = .none
                dismiss()
            }
        }
    }
}

// MARK: - Caution deletion

struct CautionDeletionSheet: View {
    let artifacts:  [Artifact]
    let projectID:  UUID
    @Environment(AppState.self) var appState
    @Environment(\.dismiss) private var dismiss

    // Per-category checkbox state
    @State private var checkedIDs: Set<UUID> = []
    @State private var deleting = false

    private var cautionItems: [Artifact] { artifacts.filter { $0.tier == .caution } }
    private var riskyItems:   [Artifact] { artifacts.filter { $0.tier == .risky   } }

    var body: some View {
        VStack(spacing: 20) {
            // Header
            HStack(spacing: 12) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.title2)
                    .foregroundStyle(.orange)
                VStack(alignment: .leading) {
                    Text("Review Items")
                        .font(.headline)
                    Text("Caution items may affect project configuration. Check each category you understand.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            Divider()

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12) {
                    if !cautionItems.isEmpty {
                        cautionSection
                    }
                    if !riskyItems.isEmpty {
                        riskyNotice
                    }
                }
            }
            .frame(maxHeight: 260)

            Spacer()

            // Actions
            HStack {
                Button("Cancel") {
                    appState.deletionFlow = .none
                    dismiss()
                }
                .keyboardShortcut(.cancelAction)

                Spacer()

                Button {
                    performDeletion()
                } label: {
                    if deleting {
                        ProgressView().scaleEffect(0.7)
                    } else {
                        Label("Delete Selected (\(checkedIDs.count))", systemImage: "trash")
                    }
                }
                .buttonStyle(.borderedProminent)
                .tint(.orange)
                .disabled(checkedIDs.isEmpty || deleting)
                .accessibilityHint("Deletes the checked caution items")
            }
        }
        .padding(24)
        .frame(width: 400)
    }

    private var cautionSection: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("⚠️ Caution")
                .font(.caption.bold())
                .foregroundStyle(.orange)

            ForEach(cautionItems) { artifact in
                HStack(alignment: .top, spacing: 8) {
                    Toggle("", isOn: Binding(
                        get: { checkedIDs.contains(artifact.id) },
                        set: { on in
                            if on { checkedIDs.insert(artifact.id) }
                            else  { checkedIDs.remove(artifact.id) }
                        }
                    ))
                    .labelsHidden()
                    .accessibilityLabel("Select \(artifact.name)")

                    VStack(alignment: .leading, spacing: 2) {
                        Text(artifact.name).font(.caption.bold()).lineLimit(1)
                        Text(artifact.reason)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                        Text(ByteCountFormatter.string(
                            fromByteCount: artifact.size, countStyle: .file
                        ))
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                    }
                }
                .padding(8)
                .background(.quinary, in: RoundedRectangle(cornerRadius: 6))
            }
        }
    }

    private var riskyNotice: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("🔴 Risky (\(riskyItems.count) items)")
                .font(.caption.bold())
                .foregroundStyle(.red)
            Text("Risky items require individual review. They will open in a separate dialog.")
                .font(.caption2)
                .foregroundStyle(.secondary)

            Button("Review Risky Items Individually") {
                // Dismiss caution sheet, open first risky item.
                appState.deletionFlow = .none
                if let first = riskyItems.first {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
                        appState.deletionFlow = .confirmRisky(artifact: first)
                    }
                }
            }
            .font(.caption)
            .buttonStyle(.bordered)
            .tint(.red)
        }
        .padding(8)
        .background(Color.red.opacity(0.06), in: RoundedRectangle(cornerRadius: 6))
    }

    private func performDeletion() {
        deleting = true
        let toDelete = artifacts.filter { checkedIDs.contains($0.id) }
        Task {
            try? await Task.sleep(for: .seconds(0.6))
            await MainActor.run {
                if let idx = appState.projects.firstIndex(where: { $0.id == projectID }) {
                    let ids = Set(toDelete.map(\.id))
                    appState.projects[idx].artifacts.removeAll { ids.contains($0.id) }
                }
                appState.deletionFlow = .none
                dismiss()
            }
        }
    }
}

// MARK: - Risky deletion

struct RiskyDeletionSheet: View {
    let artifact: Artifact
    @Environment(AppState.self) var appState
    @Environment(\.dismiss) private var dismiss
    @State private var confirmed  = false
    @State private var deleting   = false
    @State private var typed      = ""

    // User must type the file name to confirm.
    private var confirmPhrase: String { artifact.name }

    var body: some View {
        VStack(spacing: 20) {
            // Header
            HStack(spacing: 12) {
                Image(systemName: "exclamationmark.octagon.fill")
                    .font(.title2)
                    .foregroundStyle(.red)
                VStack(alignment: .leading) {
                    Text("Risky Deletion")
                        .font(.headline)
                    Text("This action may be irreversible. Proceed with care.")
                        .font(.caption)
                        .foregroundStyle(.red.opacity(0.8))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            Divider()

            // Item detail
            VStack(alignment: .leading, spacing: 6) {
                detailRow(label: "Name",  value: artifact.name)
                detailRow(label: "Path",  value: artifact.path)
                detailRow(
                    label: "Size",
                    value: ByteCountFormatter.string(fromByteCount: artifact.size, countStyle: .file)
                )
                detailRow(label: "Risk",  value: artifact.reason)
            }
            .padding(12)
            .background(Color.red.opacity(0.06), in: RoundedRectangle(cornerRadius: 8))

            // Typed confirmation
            VStack(alignment: .leading, spacing: 6) {
                Text("Type **\(confirmPhrase)** to confirm deletion:")
                    .font(.caption)
                TextField("", text: $typed)
                    .textFieldStyle(.roundedBorder)
                    .font(.caption)
                    .accessibilityLabel("Type the file name to confirm")
            }

            Spacer()

            HStack {
                Button("Cancel") {
                    appState.deletionFlow = .none
                    dismiss()
                }
                .keyboardShortcut(.cancelAction)

                Spacer()

                Button {
                    performDeletion()
                } label: {
                    if deleting {
                        ProgressView().scaleEffect(0.7)
                    } else {
                        Label("Permanently Delete", systemImage: "trash.fill")
                    }
                }
                .buttonStyle(.borderedProminent)
                .tint(.red)
                .disabled(typed != confirmPhrase || deleting)
                .accessibilityHint("Type the file name above to enable this button")
            }
        }
        .padding(24)
        .frame(width: 380)
    }

    private func detailRow(label: String, value: String) -> some View {
        HStack(alignment: .top) {
            Text(label)
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(width: 40, alignment: .leading)
            Text(value)
                .font(.caption)
                .lineLimit(2)
                .truncationMode(.middle)
        }
    }

    private func performDeletion() {
        deleting = true
        Task {
            try? await Task.sleep(for: .seconds(0.6))
            await MainActor.run {
                // Remove from all projects
                for idx in appState.projects.indices {
                    appState.projects[idx].artifacts.removeAll { $0.id == artifact.id }
                }
                appState.deletionFlow = .none
                dismiss()
            }
        }
    }
}

// MARK: - SafetyTier → DeletionTier conversion

extension SafetyTier {
    /// Maps AppState SafetyTier to TrashCoordinator DeletionTier.
    func asDeletionTier() -> DeletionTier {
        switch self {
        case .safe:    return .buildOutputs
        case .caution: return .heavyCompilation
        case .risky:   return .statefulItems
        }
    }
}

/// Returns true if any ancestor directory of `url` contains a `.git` directory.
private func isInsideGitProject(_ url: URL) -> Bool {
    var candidate = url.deletingLastPathComponent()
    while candidate.path != "/" {
        let gitDir = candidate.appendingPathComponent(".git")
        if FileManager.default.fileExists(atPath: gitDir.path) { return true }
        candidate = candidate.deletingLastPathComponent()
    }
    return false
}
