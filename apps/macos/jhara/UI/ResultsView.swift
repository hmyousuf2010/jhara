// ResultsView.swift
// Shown after scan completes.
// Top half: squarified treemap. Bottom half: project-grouped scrollable list.

import SwiftUI

struct ResultsView: View {
    @Environment(AppState.self) var appState
    @State private var selectedProjectID: UUID?
    @State private var showOrphans = false

    var body: some View {
        @Bindable var appState = appState
        VStack(spacing: 0) {
            toolbar
            Divider()

            if appState.projects.isEmpty && appState.detectedOrphans.isEmpty {
                emptyState
            } else {
                VSplitView {
                    // ── Treemap ──────────────────────────────────────────────
                    TreemapView(projects: appState.projects) { project in
                        selectedProjectID = project.id
                        // Expand the tapped project in the list.
                        if let idx = appState.projects.firstIndex(where: { $0.id == project.id }) {
                            appState.projects[idx].isExpanded = true
                        }
                    }
                    .frame(minHeight: 160, maxHeight: 240)

                    // ── Project list ─────────────────────────────────────────
                    ScrollViewReader { proxy in
                        List {
                            if !appState.detectedOrphans.isEmpty {
                                orphanSection
                            }
                            ForEach($appState.projects) { $project in
                                ProjectRow(project: $project)
                                    .id(project.id)
                                    .listRowBackground(
                                        selectedProjectID == project.id
                                        ? Color.purple.opacity(0.08)
                                        : Color.clear
                                    )
                            }
                        }
                        .listStyle(.plain)
                        .onChange(of: selectedProjectID) { _, id in
                            if let id { withAnimation { proxy.scrollTo(id, anchor: .top) } }
                        }
                    }
                }
            }
        }
    }

    // MARK: - Toolbar

    private var toolbar: some View {
        HStack(spacing: 12) {
            Button {
                appState.phase = .idle
            } label: {
                Image(systemName: "arrow.left")
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Back to scan")

            VStack(alignment: .leading, spacing: 1) {
                Text("Results")
                    .font(.headline)
                Text(
                    ByteCountFormatter.string(
                        fromByteCount: appState.totalReclaimable,
                        countStyle:    .file
                    ) + " reclaimable"
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }

            Spacer()

            if appState.orphanDetectionAvailable && !appState.detectedOrphans.isEmpty {
                Button {
                    showOrphans.toggle()
                } label: {
                    Label("Orphans", systemImage: "cpu")
                        .font(.caption)
                }
                .buttonStyle(.bordered)
                .tint(.orange)
                .accessibilityLabel("Show Intel orphan artefacts")
            }

            Button {
                appState.startScan(
                    roots: [FileManager.default.homeDirectoryForCurrentUser.path]
                )
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Re-scan")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    // MARK: - Orphan section

    private var orphanSection: some View {
        Section {
            DisclosureGroup("Intel Orphan Artefacts (\(appState.detectedOrphans.count))") {
                ForEach(appState.detectedOrphans) { artifact in
                    ArtifactRow(artifact: artifact)
                }
                Button("Remove All Orphans") {
                    appState.deletionFlow = .confirmSafe(
                        artifacts: appState.detectedOrphans,
                        projectID: UUID()
                    )
                }
                .buttonStyle(.borderedProminent)
                .tint(.orange)
                .padding(.top, 4)
            }
        } header: {
            Label("Apple Silicon", systemImage: "cpu")
                .font(.caption.bold())
                .foregroundStyle(.orange)
        }
    }

    // MARK: - Empty state

    private var emptyState: some View {
        VStack(spacing: 12) {
            Spacer()
            Image(systemName: "checkmark.seal.fill")
                .font(.system(size: 44))
                .foregroundStyle(.green)
            Text("Nothing to clean!")
                .font(.title3.bold())
            Text("No reclaimable artefacts were found in the scanned directories.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Spacer()
        }
        .padding(32)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Scan complete. Nothing to clean.")
    }
}

// MARK: - ProjectRow

struct ProjectRow: View {
    @Binding var project: ProjectResult
    @Environment(AppState.self) var appState

    var body: some View {
        DisclosureGroup(isExpanded: $project.isExpanded) {
            artifactList
        } label: {
            projectLabel
        }
        .padding(.vertical, 2)
    }

    private var projectLabel: some View {
        HStack(spacing: 10) {
            Image(systemName: "folder.fill")
                .foregroundStyle(.purple.opacity(0.7))
                .frame(width: 18)

            VStack(alignment: .leading, spacing: 1) {
                Text(project.name)
                    .font(.subheadline.bold())
                    .lineLimit(1)
                Text(project.rootPath)
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }

            Spacer()

            VStack(alignment: .trailing, spacing: 1) {
                Text(ByteCountFormatter.string(
                    fromByteCount: project.totalReclaimable,
                    countStyle:    .file
                ))
                .font(.caption.bold())

                TierBadgeRow(project: project)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(project.name), \(ByteCountFormatter.string(fromByteCount: project.totalReclaimable, countStyle: .file)) reclaimable")
    }

    private var artifactList: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(project.artifacts) { artifact in
                ArtifactRow(artifact: artifact)
                    .padding(.leading, 8)
            }

            actionButtons
                .padding(.top, 8)
                .padding(.leading, 8)
                .padding(.bottom, 4)
        }
    }

    @ViewBuilder
    private var actionButtons: some View {
        HStack(spacing: 8) {
            // "Remove Safe Items" — one-click with summary
            if !project.safeArtifacts.isEmpty {
                Button {
                    appState.deletionFlow = .confirmSafe(
                        artifacts: project.safeArtifacts,
                        projectID: project.id
                    )
                } label: {
                    Label("Remove Safe", systemImage: "trash")
                        .font(.caption)
                }
                .buttonStyle(.borderedProminent)
                .tint(.green)
                .accessibilityHint("Removes \(project.safeArtifacts.count) safe items after confirmation")
            }

            // "Review Caution/Risky"
            if !project.cautionArtifacts.isEmpty || !project.riskyArtifacts.isEmpty {
                Button {
                    // Open caution flow first; risky handled per-item inside.
                    appState.deletionFlow = .confirmCaution(
                        artifacts: project.cautionArtifacts + project.riskyArtifacts,
                        projectID: project.id
                    )
                } label: {
                    Label("Review All", systemImage: "eye")
                        .font(.caption)
                }
                .buttonStyle(.bordered)
                .tint(.orange)
                .accessibilityHint("Review caution and risky items before removal")
            }
        }
    }
}

// MARK: - ArtifactRow

struct ArtifactRow: View {
    let artifact: Artifact

    var body: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(tierColor)
                .frame(width: 6, height: 6)
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 1) {
                Text(artifact.name)
                    .font(.caption)
                    .lineLimit(1)
                Text(artifact.reason)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer()

            Text(ByteCountFormatter.string(fromByteCount: artifact.size, countStyle: .file))
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .padding(.vertical, 3)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(artifact.name), \(ByteCountFormatter.string(fromByteCount: artifact.size, countStyle: .file)), \(artifact.tier.rawValue)")
    }

    private var tierColor: Color {
        switch artifact.tier {
        case .safe:    return .green
        case .caution: return .orange
        case .risky:   return .red
        }
    }
}

// MARK: - TierBadgeRow

struct TierBadgeRow: View {
    let project: ProjectResult

    var body: some View {
        HStack(spacing: 3) {
            if !project.safeArtifacts.isEmpty {
                tierBadge(count: project.safeArtifacts.count,    color: .green)
            }
            if !project.cautionArtifacts.isEmpty {
                tierBadge(count: project.cautionArtifacts.count, color: .orange)
            }
            if !project.riskyArtifacts.isEmpty {
                tierBadge(count: project.riskyArtifacts.count,   color: .red)
            }
        }
    }

    private func tierBadge(count: Int, color: Color) -> some View {
        Text("\(count)")
            .font(.system(size: 9, weight: .bold))
            .foregroundStyle(.white)
            .padding(.horizontal, 4)
            .padding(.vertical, 1)
            .background(color, in: Capsule())
    }
}
