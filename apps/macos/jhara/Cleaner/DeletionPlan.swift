import Foundation

// MARK: - Deletion Tier

/// Represents the safety tier of a deletable artifact.
/// Lower raw values are deleted first (safest recovery path first).
public enum DeletionTier: Int, Comparable, CaseIterable, Sendable {
    case pureCaches       = 1   // node_modules/.cache, ~/.npm/_cacache  — sub-minute recovery
    case buildOutputs     = 2   // dist/, .next/, build/                 — build-script recovery
    case dependencyDirs   = 3   // node_modules/, vendor/, .venv/        — package-manager recovery
    case heavyCompilation = 4   // Rust target/, Xcode DerivedData       — full recompile
    case statefulItems    = 5   // Docker volumes, Conda envs            — explicit per-item confirm

    public static func < (lhs: DeletionTier, rhs: DeletionTier) -> Bool {
        lhs.rawValue < rhs.rawValue
    }

    public var displayName: String {
        switch self {
        case .pureCaches:       return "Pure Caches"
        case .buildOutputs:     return "Build Outputs"
        case .dependencyDirs:   return "Dependency Directories"
        case .heavyCompilation: return "Heavy Compilation Outputs"
        case .statefulItems:    return "Stateful Items"
        }
    }

    public var recoveryDescription: String {
        switch self {
        case .pureCaches:       return "Sub-minute recovery (auto-regenerated)"
        case .buildOutputs:     return "Recovered by re-running build scripts"
        case .dependencyDirs:   return "Recovered via package manager (npm install, pip install, etc.)"
        case .heavyCompilation: return "Requires full recompile (may take minutes)"
        case .statefulItems:    return "Explicit per-item confirmation required; manual recovery"
        }
    }

    /// Whether items in this tier require an individual explicit confirmation
    /// before deletion, regardless of a top-level user approval.
    public var requiresPerItemConfirmation: Bool {
        self == .statefulItems
    }
}

// MARK: - Scan Item (input from ScanTree)

/// A single item discovered by ScanTree and queued for potential deletion.
public struct ScanItem: Identifiable, Sendable {
    public let id: UUID
    public let url: URL
    public let tier: DeletionTier
    /// Byte size on disk. `nil` if not yet measured.
    public var sizeOnDisk: Int64?
    /// Number of contained filesystem entries. `nil` if not yet counted.
    public var fileCount: Int?
    /// Whether this item resides inside a `.git`-bearing project directory.
    public var isInsideGitProject: Bool
    /// Cloud-sync paths (iCloud Drive, Dropbox, etc.) must be guarded.
    public var isCloudSyncedPath: Bool

    public init(
        id: UUID = UUID(),
        url: URL,
        tier: DeletionTier,
        sizeOnDisk: Int64? = nil,
        fileCount: Int? = nil,
        isInsideGitProject: Bool = false,
        isCloudSyncedPath: Bool = false
    ) {
        self.id = id
        self.url = url
        self.tier = tier
        self.sizeOnDisk = sizeOnDisk
        self.fileCount = fileCount
        self.isInsideGitProject = isInsideGitProject
        self.isCloudSyncedPath = isCloudSyncedPath
    }
}

// MARK: - Deletion Stage

/// A group of ScanItems sharing the same DeletionTier, ready to be executed together.
public struct DeletionStage: Identifiable, Sendable {
    public let id: UUID
    public let tier: DeletionTier
    public private(set) var items: [ScanItem]

    /// Aggregate bytes that will be freed if this stage completes.
    public var totalSizeOnDisk: Int64 {
        items.compactMap(\.sizeOnDisk).reduce(0, +)
    }

    public var totalFileCount: Int {
        items.compactMap(\.fileCount).reduce(0, +)
    }

    public init(tier: DeletionTier, items: [ScanItem] = []) {
        self.id = UUID()
        self.tier = tier
        self.items = items
    }

    mutating func append(_ item: ScanItem) {
        items.append(item)
    }
}

// MARK: - Git Status Check Result

public enum GitStatusResult: Sendable {
    case clean
    case dirty(statusOutput: String)
    case notAGitRepo
    case checkFailed(Error)
}

// MARK: - Deletion Plan

/// Produced from ScanTree output. Holds an ordered array of `DeletionStage`s
/// (Level 1 → Level 5) plus pre-flight metadata.
public struct DeletionPlan: Sendable {

    // MARK: Properties

    /// Stages ordered from safest (tier 1) to most sensitive (tier 5).
    public private(set) var stages: [DeletionStage]

    /// Items that were excluded from the plan due to safety rules
    /// (dirty git repos, cloud-synced paths, etc.).
    public private(set) var excludedItems: [(item: ScanItem, reason: String)]

    /// Wall-clock date when the plan was built.
    public let createdAt: Date

    // MARK: Computed Aggregates

    public var allItems: [ScanItem] { stages.flatMap(\.items) }

    public var totalSizeOnDisk: Int64 { stages.map(\.totalSizeOnDisk).reduce(0, +) }

    public var totalFileCount: Int { stages.map(\.totalFileCount).reduce(0, +) }

    public var stageCount: Int { stages.count }

    public var isEmpty: Bool { allItems.isEmpty }

    // MARK: Init

    init(stages: [DeletionStage],
         excludedItems: [(item: ScanItem, reason: String)],
         createdAt: Date) {
        self.stages = stages
        self.excludedItems = excludedItems
        self.createdAt = createdAt
    }

    // MARK: - Builder

    /// Classifies `scanItems` into a `DeletionPlan`, running git safety checks
    /// and cloud-sync guards along the way.
    ///
    /// - Parameters:
    ///   - scanItems:  Raw items emitted by ScanTree.
    ///   - gitChecker: Injected dependency; defaults to `LiveGitChecker`.
    /// - Returns: A fully ordered `DeletionPlan`.
    public static func build(
        from scanItems: [ScanItem],
        gitChecker: GitChecking = LiveGitChecker()
    ) async -> DeletionPlan {

        var buckets: [DeletionTier: DeletionStage] = Dictionary(
            uniqueKeysWithValues: DeletionTier.allCases.map { ($0, DeletionStage(tier: $0)) }
        )
        var excluded: [(ScanItem, String)] = []

        for item in scanItems {
            // --- Cloud-sync guard ---
            if item.isCloudSyncedPath {
                excluded.append((item, "Resides in a cloud-synced path; skipped to avoid sync conflicts."))
                continue
            }

            // --- Git safety check ---
            if item.isInsideGitProject {
                let projectRoot = gitProjectRoot(for: item.url)
                let status = await gitChecker.status(at: projectRoot ?? item.url)
                switch status {
                case .dirty(let output):
                    excluded.append((
                        item,
                        "Git repo has uncommitted changes — explicit confirmation required.\n\(output)"
                    ))
                    continue
                case .checkFailed(let error):
                    excluded.append((item, "Git status check failed: \(error.localizedDescription)"))
                    continue
                case .clean, .notAGitRepo:
                    break // safe to proceed
                }
            }

            buckets[item.tier]!.append(item)
        }

        // Sort stages by tier (ascending = safest first), drop empty stages.
        let orderedStages = DeletionTier.allCases
            .compactMap { buckets[$0] }
            .filter { !$0.items.isEmpty }

        return DeletionPlan(
            stages: orderedStages,
            excludedItems: excluded,
            createdAt: Date()
        )
    }

    // MARK: - Helpers

    /// Returns the stage for a given tier, or `nil` if no items exist in that tier.
    public func stage(for tier: DeletionTier) -> DeletionStage? {
        stages.first { $0.tier == tier }
    }

    /// Returns a human-readable pre-deletion summary.
    public func preDeletionSummary() -> String {
        var lines: [String] = [
            "── Deletion Plan (\(ISO8601DateFormatter().string(from: createdAt))) ──",
            "Stages: \(stageCount)   Items: \(allItems.count)   Size: \(formattedBytes(totalSizeOnDisk))",
            ""
        ]

        for stage in stages {
            lines.append("  Level \(stage.tier.rawValue) — \(stage.tier.displayName)")
            lines.append("    \(stage.items.count) item(s)  •  \(formattedBytes(stage.totalSizeOnDisk))")
            lines.append("    Recovery: \(stage.tier.recoveryDescription)")
            for item in stage.items {
                let size = item.sizeOnDisk.map { " (\(formattedBytes($0)))" } ?? ""
                lines.append("      • \(item.url.path)\(size)")
            }
            lines.append("")
        }

        if !excludedItems.isEmpty {
            lines.append("  ⚠︎  Excluded (\(excludedItems.count)):")
            for (item, reason) in excludedItems {
                lines.append("      • \(item.url.path)")
                lines.append("        Reason: \(reason)")
            }
        }

        return lines.joined(separator: "\n")
    }
}

// MARK: - Git Checker Protocol + Live Implementation

public protocol GitChecking: Sendable {
    func status(at url: URL) async -> GitStatusResult
}

public struct LiveGitChecker: GitChecking {
    public init() {}

    public func status(at url: URL) async -> GitStatusResult {
        guard url.isFileURL else { return .notAGitRepo }
        let path = url.path
        guard FileManager.default.fileExists(atPath: (path as NSString).appendingPathComponent(".git")) else {
            return .notAGitRepo
        }

        return await withCheckedContinuation { continuation in
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
            process.arguments = ["-C", path, "status", "--porcelain"]
            let pipe = Pipe()
            process.standardOutput = pipe
            process.standardError = Pipe()

            do {
                try process.run()
                process.waitUntilExit()
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                let output = String(data: data, encoding: .utf8) ?? ""
                if output.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    continuation.resume(returning: .clean)
                } else {
                    continuation.resume(returning: .dirty(statusOutput: output))
                }
            } catch {
                continuation.resume(returning: .checkFailed(error))
            }
        }
    }
}

// MARK: - Private Helpers

private func gitProjectRoot(for url: URL) -> URL? {
    var candidate = url.deletingLastPathComponent()
    while candidate.path != "/" {
        let gitDir = candidate.appendingPathComponent(".git")
        if FileManager.default.fileExists(atPath: gitDir.path) { return candidate }
        candidate = candidate.deletingLastPathComponent()
    }
    return nil
}

private func formattedBytes(_ bytes: Int64) -> String {
    ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
}
