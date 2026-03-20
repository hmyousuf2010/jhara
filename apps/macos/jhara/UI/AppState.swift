// AppState.swift
// Central @Observable state object shared across the whole SwiftUI tree.

import Foundation
import Observation

// MARK: - Safety Tier

public enum SafetyTier: String, CaseIterable, Sendable {
    case safe    = "Safe"
    case caution = "Caution"
    case risky   = "Risky"

    var color: String {
        switch self {
        case .safe:    return "SafeGreen"
        case .caution: return "CautionAmber"
        case .risky:   return "RiskyRed"
        }
    }
}

// MARK: - Artifact

public struct Artifact: Identifiable, Sendable {
    public let id:           UUID
    public let path:         String
    public let name:         String
    public let size:         Int64
    public let tier:         SafetyTier
    public let reason:       String
    public let lastActivity: Date
    public var isSelected:   Bool = false
}

// MARK: - ProjectResult

public struct ProjectResult: Identifiable, Sendable {
    public let id:        UUID
    public let name:      String
    public let rootPath:  String
    public var artifacts: [Artifact]
    public var isExpanded: Bool = false

    public var totalReclaimable: Int64 { artifacts.reduce(0) { $0 + $1.size } }
    public var safeArtifacts:    [Artifact] { artifacts.filter { $0.tier == .safe    } }
    public var cautionArtifacts: [Artifact] { artifacts.filter { $0.tier == .caution } }
    public var riskyArtifacts:   [Artifact] { artifacts.filter { $0.tier == .risky   } }
}

// MARK: - FFI JSON Decodable types
// These match the JSON emitted by jhara_core_projects_results_json (Rust DetectedProject)

private struct FoundArtifactDecoded: Decodable {
    let absolute_path: String
    let safety_tier: String   // "safe" | "caution" | "risky" | "blocked"
    let physical_size_bytes: Int64
    let recovery_command: String?
    let is_ghost: Bool
}

private struct DetectedProjectDecoded: Decodable {
    let root_path: String
    let artifacts: [FoundArtifactDecoded]
}

// MARK: - Scan Phase

public enum ScanPhase: Equatable {
    case idle
    case scanning(progress: Double)
    case done
    case failed(String)
}

// MARK: - Deletion Flow State

public enum DeletionFlow {
    case none
    case confirmSafe(artifacts: [Artifact],    projectID: UUID)
    case confirmCaution(artifacts: [Artifact], projectID: UUID)
    case confirmRisky(artifact: Artifact)
}

// MARK: - AppState

@Observable
@MainActor
public final class AppState {

    // ── Scan ──────────────────────────────────────────────────────────────────
    public var phase:    ScanPhase = .idle
    public var projects: [ProjectResult] = []

    // ── Global caches (npm, cargo registry, CocoaPods, etc.) ─────────────────
    /// Artifacts detected in the user's home directory, not tied to any project.
    public var globalCaches: [Artifact] = []

    // ── Apple Silicon orphan detection ────────────────────────────────────────
    public var orphanDetectionAvailable: Bool = isAppleSilicon()
    public var detectedOrphans: [Artifact] = []

    // ── Deletion ──────────────────────────────────────────────────────────────
    public var deletionFlow:       DeletionFlow = .none
    public var deletionInProgress: Bool = false
    public var lastDeletionError:  String? = nil

    // ── Computed ─────────────────────────────────────────────────────────────
    public var totalReclaimable: Int64 {
        projects.reduce(0) { $0 + $1.totalReclaimable }
        + globalCaches.reduce(0) { $0 + $1.size }
    }

    public var isScanning: Bool {
        if case .scanning = phase { return true }
        return false
    }

    // ── Scan trigger ─────────────────────────────────────────────────────────
    private var coordinator: ScanCoordinator?
    private var fsEventsMonitor: FSEventsMonitor?

    @MainActor
    public func startScan(roots: [String]) {
        guard !isScanning else { return }
        phase    = .scanning(progress: 0)
        projects = []
        globalCaches = []
        let skipList = iCloudGuard.buildSkipList(
            homeURL: FileManager.default.homeDirectoryForCurrentUser
        )
        Task { await runScan(roots: roots, skipList: skipList) }
    }

    @MainActor
    public func cancelScan() {
        Task { await coordinator?.cancel() }
    }

    // MARK: - Private scan runner

    private func runScan(roots: [String], skipList: [String]) async {
        let coord = ScanCoordinator()
        coordinator = coord

        var received = 0

        for await event in await coord.scan(roots: roots, skipList: skipList) {
            switch event {
            case .batch(let nodes):
                received += nodes.count
                let progress = min(Double(received) / 50_000, 0.95)
                await MainActor.run { phase = .scanning(progress: progress) }

            case .completed:
                await MainActor.run {
                    guard let handle = coord.currentHandle else {
                        phase = .failed("Scan handle lost before classification could run.")
                        return
                    }

                    // ── Projects: ask Rust engine for classified project JSON ──────────
                    guard let jsonPtr = jhara_core_projects_results_json(handle) else {
                        // Rust engine returned null — this is a hard failure.
                        // Do NOT fall back to a Swift stub; surface the error so bugs
                        // are immediately visible during development (Option A).
                        phase = .failed(
                            "Rust classifier returned no results. " +
                            "Check that the scan root contains recognisable projects " +
                            "(Cargo.toml, package.json, go.mod, etc.)."
                        )
                        return
                    }
                    let jsonStr = String(cString: jsonPtr)
                    jhara_core_string_free(jsonPtr)

                    guard let data = jsonStr.data(using: .utf8),
                          let decoded = try? JSONDecoder().decode([DetectedProjectDecoded].self, from: data)
                    else {
                        phase = .failed("Failed to decode project JSON from Rust engine.")
                        return
                    }

                    projects = decoded.map { dp in
                        let arts = dp.artifacts.compactMap { fa -> Artifact? in
                            // Blocked artifacts are never shown in the UI.
                            guard fa.safety_tier != "blocked" else { return nil }
                            let tier: SafetyTier = {
                                switch fa.safety_tier {
                                case "caution": return .caution
                                case "risky":   return .risky
                                default:        return .safe
                                }
                            }()
                            return Artifact(
                                id:           UUID(),
                                path:         fa.absolute_path,
                                name:         URL(fileURLWithPath: fa.absolute_path).lastPathComponent,
                                size:         fa.physical_size_bytes,
                                tier:         tier,
                                reason:       fa.recovery_command ?? "Reconstructable artifact",
                                lastActivity: Date()
                            )
                        }
                        return ProjectResult(
                            id:       UUID(),
                            name:     URL(fileURLWithPath: dp.root_path).lastPathComponent,
                            rootPath: dp.root_path,
                            artifacts: arts
                        )
                    }.sorted { $0.totalReclaimable > $1.totalReclaimable }

                    // ── Global caches: ~/.npm, ~/.cargo/registry, etc. ────────────
                    let homeDir = FileManager.default.homeDirectoryForCurrentUser.path
                    homeDir.withCString { homeCStr in
                        if let gcPtr = jhara_core_global_caches_json(handle, homeCStr),
                           let gcStr = String(validatingUTF8: gcPtr),
                           let gcData = gcStr.data(using: .utf8),
                           let gcDecoded = try? JSONDecoder().decode([FoundArtifactDecoded].self, from: gcData) {
                            jhara_core_string_free(gcPtr)
                            globalCaches = gcDecoded.compactMap { fa -> Artifact? in
                                guard fa.safety_tier != "blocked" else { return nil }
                                let tier: SafetyTier = {
                                    switch fa.safety_tier {
                                    case "caution": return .caution
                                    case "risky":   return .risky
                                    default:        return .safe
                                    }
                                }()
                                return Artifact(
                                    id:           UUID(),
                                    path:         fa.absolute_path,
                                    name:         URL(fileURLWithPath: fa.absolute_path).lastPathComponent,
                                    size:         fa.physical_size_bytes,
                                    tier:         tier,
                                    reason:       fa.recovery_command ?? "Global developer tool cache",
                                    lastActivity: Date()
                                )
                            }.sorted { $0.size > $1.size }
                        }
                    }

                    // ── Orphan detection (Apple Silicon only) ─────────────────────
                    // NOTE: orphan detection uses a separate simple path-prefix check
                    // that does not require Rust classification — kept in Swift intentionally.
                    if orphanDetectionAvailable {
                        // Orphans are detected from the scan handle, not nodeBuffer.
                        // For now keep the existing approach via a direct path check —
                        // this will be migrated to a Rust FFI call in a follow-up.
                        detectedOrphans = []
                    }

                    phase = .done

                    // Phase 6: Start watching for filesystem changes
                    let watchRoots = roots
                    Task {
                        let monitor = FSEventsMonitor(latency: 30.0)
                        await MainActor.run { self.fsEventsMonitor = monitor }
                        await monitor.start(watching: watchRoots)
                        for await batch in await monitor.changes {
                            let affectedRoots = batch.minimalCoveringAncestorSet()
                                .filter { changed in watchRoots.contains(where: { changed.hasPrefix($0) }) }
                            if !affectedRoots.isEmpty {
                                await MainActor.run { self.startScan(roots: affectedRoots) }
                            }
                        }
                    }
                }

            case .cancelled:
                await MainActor.run { phase = .idle }

            case .failed(let err):
                await MainActor.run { phase = .failed(err.localizedDescription) }
            }
        }
    }

    // MARK: - Apple Silicon orphan detection
    // TODO: Migrate to jhara_core_orphan_scan_json once that FFI function is added.
    // For now this is intentionally kept minimal — no Swift classification logic.
    private func detectOrphans() -> [Artifact] {
        // Orphan detection requires access to the raw scan results.
        // Deferred until a dedicated FFI function is available.
        return []
    }
}

// MARK: - Helpers

private func isAppleSilicon() -> Bool {
    var size = 0
    sysctlbyname("hw.optional.arm64", nil, &size, nil, 0)
    var value: Int32 = 0
    sysctlbyname("hw.optional.arm64", &value, &size, nil, 0)
    return value == 1
}
