import Foundation

// MARK: - Deletion Result

public enum ItemDeletionResult: Sendable {
    case trashed(url: URL, bytesReclaimed: Int64)
    case skipped(url: URL, reason: String)
    case failed(url: URL, error: Error)
}

// MARK: - Deletion Progress

public struct DeletionProgress: Sendable {
    public let currentItem: URL
    public let completedItems: Int
    public let totalItems: Int
    public let bytesReclaimedSoFar: Int64
    public let currentTier: DeletionTier
    public let currentTierCompleted: Int
    public let currentTierTotal: Int

    public var fractionCompleted: Double {
        totalItems == 0 ? 0 : Double(completedItems) / Double(totalItems)
    }
}

// MARK: - Deletion Summary

public struct DeletionSummary: Sendable {
    public let startedAt: Date
    public let finishedAt: Date
    public let results: [ItemDeletionResult]
    public let wasInterrupted: Bool

    public var duration: TimeInterval { finishedAt.timeIntervalSince(startedAt) }

    public var trashedCount: Int {
        results.filter { if case .trashed = $0 { return true }; return false }.count
    }

    public var skippedCount: Int {
        results.filter { if case .skipped = $0 { return true }; return false }.count
    }

    public var failedCount: Int {
        results.filter { if case .failed = $0 { return true }; return false }.count
    }

    public var totalBytesReclaimed: Int64 {
        results.compactMap {
            if case .trashed(_, let b) = $0 { return b }
            return nil
        }.reduce(0, +)
    }

    /// Human-readable post-deletion summary.
    public func postDeletionSummary() -> String {
        let fmt = ISO8601DateFormatter()
        var lines: [String] = [
            "── Post-Deletion Summary ──",
            "Started : \(fmt.string(from: startedAt))",
            "Finished: \(fmt.string(from: finishedAt))  (\(String(format: "%.1f", duration))s)",
            wasInterrupted ? "⚠︎  Run was INTERRUPTED — partial completion recorded." : "✓  Run completed normally.",
            "",
            "  Trashed : \(trashedCount)",
            "  Skipped : \(skippedCount)",
            "  Failed  : \(failedCount)",
            "  Reclaimed: \(ByteCountFormatter.string(fromByteCount: totalBytesReclaimed, countStyle: .file))",
        ]

        if failedCount > 0 {
            lines.append("")
            lines.append("  Failed items:")
            for r in results {
                if case .failed(let url, let err) = r {
                    lines.append("    • \(url.path): \(err.localizedDescription)")
                }
            }
        }

        return lines.joined(separator: "\n")
    }
}

// MARK: - Confirmation Handler

/// Called by TrashCoordinator when confirmation is required before deleting an item.
/// Return `true` to proceed, `false` to skip the item.
public typealias ConfirmationHandler = @Sendable (ScanItem) async -> Bool

// MARK: - TrashCoordinator

/// Executes a `DeletionPlan` stage by stage, trashing items via `FileManager.default.trashItem()`.
///
/// Features:
/// - Per-item timeout with graceful skip on expiry.
/// - Batch enumeration of large directories (10 000+ files) to avoid Finder hangs.
/// - Cloud-sync path guard (secondary check at execution time).
/// - Per-item confirmation callback for Level-5 stateful items.
/// - Cooperative cancellation: caller cancels the `Task` to interrupt cleanly.
/// - Real-time progress via `AsyncStream<DeletionProgress>`.
public actor TrashCoordinator {

    // MARK: - Configuration

    public struct Configuration: Sendable {
        /// Timeout per item before it is skipped. Default: 30 s.
        public var itemTimeout: TimeInterval = 30
        /// Directories with ≥ this many files are enumerated in batches. Default: 10 000.
        public var largeDirectoryThreshold: Int = 10_000
        /// Number of entries enumerated per batch for large directories. Default: 500.
        public var enumerationBatchSize: Int = 500
        /// If `true`, run in dry-run mode — log actions without touching the filesystem.
        public var dryRun: Bool = false

        public init() {}
    }

    // MARK: - State

    private let configuration: Configuration
    private let confirmationHandler: ConfirmationHandler
    private let fileManager: FileManager

    private var _results: [ItemDeletionResult] = []
    private var _startedAt: Date?
    private var _interrupted: Bool = false

    // MARK: - Init

    public init(
        configuration: Configuration = Configuration(),
        confirmationHandler: @escaping ConfirmationHandler,
        fileManager: FileManager = .default
    ) {
        self.configuration = configuration
        self.confirmationHandler = confirmationHandler
        self.fileManager = fileManager
    }

    // MARK: - Public API

    /// Executes the plan and streams progress updates.
    ///
    /// - Parameters:
    ///   - plan: A fully-built `DeletionPlan`.
    ///   - progressHandler: Called on every item completion with the latest `DeletionProgress`.
    /// - Returns: A `DeletionSummary` once all stages complete (or are interrupted).
    @discardableResult
    public func execute(
        plan: DeletionPlan,
        progressHandler: (@Sendable (DeletionProgress) -> Void)? = nil
    ) async -> DeletionSummary {
        _results = []
        _startedAt = Date()
        _interrupted = false

        let allItems = plan.stages.flatMap(\.items)
        let totalItems = allItems.count
        var completedItems = 0
        var bytesReclaimed: Int64 = 0

        stageLoop: for stage in plan.stages {
            // Check for cooperative cancellation between stages.
            if Task.isCancelled {
                _interrupted = true
                break stageLoop
            }

            let tierTotal = stage.items.count

            for (tierIndex, item) in stage.items.enumerated() {
                // Cooperative cancellation within a stage.
                if Task.isCancelled {
                    _interrupted = true
                    break stageLoop
                }

                // ── Level 5: per-item explicit confirmation ──
                if stage.tier.requiresPerItemConfirmation {
                    let confirmed = await confirmationHandler(item)
                    guard confirmed else {
                        let result = ItemDeletionResult.skipped(url: item.url, reason: "User declined confirmation for stateful item.")
                        _results.append(result)
                        completedItems += 1
                        progressHandler?(makeProgress(
                            currentItem: item.url,
                            completed: completedItems,
                            total: totalItems,
                            bytesReclaimed: bytesReclaimed,
                            tier: stage.tier,
                            tierCompleted: tierIndex + 1,
                            tierTotal: tierTotal
                        ))
                        continue
                    }
                }

                // ── Cloud-sync secondary guard ──
                if item.isCloudSyncedPath {
                    let result = ItemDeletionResult.skipped(
                        url: item.url,
                        reason: "Cloud-synced path guard triggered at execution time."
                    )
                    _results.append(result)
                    completedItems += 1
                    continue
                }

                // ── Trash the item (with timeout) ──
                let result = await trashItemWithTimeout(item)
                if case .trashed(_, let bytes) = result { bytesReclaimed += bytes }
                _results.append(result)
                completedItems += 1

                progressHandler?(makeProgress(
                    currentItem: item.url,
                    completed: completedItems,
                    total: totalItems,
                    bytesReclaimed: bytesReclaimed,
                    tier: stage.tier,
                    tierCompleted: tierIndex + 1,
                    tierTotal: tierTotal
                ))
            }
        }

        return DeletionSummary(
            startedAt: _startedAt ?? Date(),
            finishedAt: Date(),
            results: _results,
            wasInterrupted: _interrupted
        )
    }

    // MARK: - Partial Completion Recovery

    /// Returns items from the plan that were NOT yet processed (for resuming an interrupted run).
    public func remainingItems(from plan: DeletionPlan) -> [ScanItem] {
        let processedURLs = Set(_results.map(\.url))
        return plan.allItems.filter { !processedURLs.contains($0.url) }
    }

    /// Re-executes only the items that were not yet processed in a previous (interrupted) run.
    @discardableResult
    public func resume(
        from plan: DeletionPlan,
        previousSummary: DeletionSummary,
        progressHandler: (@Sendable (DeletionProgress) -> Void)? = nil
    ) async -> DeletionSummary {
        let remaining = remainingItems(from: plan)
        guard !remaining.isEmpty else { return previousSummary }

        // Build a reduced plan from remaining items.
        let resumePlan = DeletionPlan.buildSynchronously(from: remaining)
        let newSummary = await execute(plan: resumePlan, progressHandler: progressHandler)

        // Merge summaries.
        return DeletionSummary(
            startedAt: previousSummary.startedAt,
            finishedAt: newSummary.finishedAt,
            results: previousSummary.results + newSummary.results,
            wasInterrupted: newSummary.wasInterrupted
        )
    }

    // MARK: - Private Helpers

    private func trashItemWithTimeout(_ item: ScanItem) async -> ItemDeletionResult {
        if configuration.dryRun {
            let size = item.sizeOnDisk ?? 0
            return .trashed(url: item.url, bytesReclaimed: size)
        }

        // Run the blocking trash call in a detached task so we can impose a timeout.
        let url = item.url
        let fm = fileManager
        let isLarge = (item.fileCount ?? 0) >= configuration.largeDirectoryThreshold
        let batchSize = configuration.enumerationBatchSize
        let timeout = configuration.itemTimeout

        return await withTaskGroup(of: ItemDeletionResult.self, returning: ItemDeletionResult.self) { group in
            group.addTask {
                if isLarge {
                    return await Self.enumerateAndTrashInBatches(url: url, fileManager: fm, batchSize: batchSize)
                } else {
                    return Self.trashSingle(url: url, fileManager: fm)
                }
            }

            group.addTask {
                try? await Task.sleep(nanoseconds: UInt64(timeout * 1_000_000_000))
                return .skipped(url: url, reason: "Timeout exceeded (\(timeout)s).")
            }

            // First result wins; cancel the other.
            let result = await group.next() ?? .skipped(url: url, reason: "Task group produced no result.")
            group.cancelAll()
            return result
        }
    }

    private static func trashSingle(url: URL, fileManager: FileManager) -> ItemDeletionResult {
        let size = (try? url.resourceValues(forKeys: [.fileSizeKey]))?.fileSize.map(Int64.init) ?? 0
        do {
            var resultURL: NSURL?
            try fileManager.trashItem(at: url, resultingItemURL: &resultURL)
            return .trashed(url: url, bytesReclaimed: size)
        } catch {
            return .failed(url: url, error: error)
        }
    }

    /// For large directories: enumerate contents in batches to keep Finder
    /// progress responsive, then trash the directory itself.
    private static func enumerateAndTrashInBatches(
        url: URL,
        fileManager: FileManager,
        batchSize: Int
    ) async -> ItemDeletionResult {
        guard let enumerator = fileManager.enumerator(
            at: url,
            includingPropertiesForKeys: [.fileSizeKey],
            options: [.skipsHiddenFiles]
        ) else {
            return trashSingle(url: url, fileManager: fileManager)
        }

        // Enumerate file sizes in batches; yield between batches for cooperative scheduling.
        var totalSize: Int64 = 0
        var batch: [URL] = []
        batch.reserveCapacity(batchSize)

        for case let fileURL as URL in enumerator {
            batch.append(fileURL)
            if batch.count >= batchSize {
                totalSize += batch.reduce(Int64(0)) { acc, u in
                    let s = (try? u.resourceValues(forKeys: [.fileSizeKey]))?.fileSize ?? 0
                    return acc + Int64(s)
                }
                batch.removeAll(keepingCapacity: true)
                await Task.yield() // cooperate with the timeout task
            }
        }
        // Handle remainder
        totalSize += batch.reduce(Int64(0)) { acc, u in
            let s = (try? u.resourceValues(forKeys: [.fileSizeKey]))?.fileSize ?? 0
            return acc + Int64(s)
        }

        // Now trash the top-level directory.
        do {
            var resultURL: NSURL?
            try fileManager.trashItem(at: url, resultingItemURL: &resultURL)
            return .trashed(url: url, bytesReclaimed: totalSize)
        } catch {
            return .failed(url: url, error: error)
        }
    }

    private func makeProgress(
        currentItem: URL,
        completed: Int,
        total: Int,
        bytesReclaimed: Int64,
        tier: DeletionTier,
        tierCompleted: Int,
        tierTotal: Int
    ) -> DeletionProgress {
        DeletionProgress(
            currentItem: currentItem,
            completedItems: completed,
            totalItems: total,
            bytesReclaimedSoFar: bytesReclaimed,
            currentTier: tier,
            currentTierCompleted: tierCompleted,
            currentTierTotal: tierTotal
        )
    }
}

// MARK: - DeletionPlan: Synchronous Builder (for resume support)

extension DeletionPlan {
    /// Synchronous plan builder used internally (no git checks; items already vetted).
    static func buildSynchronously(from items: [ScanItem]) -> DeletionPlan {
        var buckets: [DeletionTier: DeletionStage] = Dictionary(
            uniqueKeysWithValues: DeletionTier.allCases.map { ($0, DeletionStage(tier: $0)) }
        )
        for item in items { buckets[item.tier]!.append(item) }
        let stages = DeletionTier.allCases
            .compactMap { buckets[$0] }
            .filter { !$0.items.isEmpty }
        return DeletionPlan(stages: stages, excludedItems: [], createdAt: Date())
    }

}

// MARK: - ItemDeletionResult URL helper

private extension ItemDeletionResult {
    var url: URL {
        switch self {
        case .trashed(let u, _): return u
        case .skipped(let u, _): return u
        case .failed(let u, _):  return u
        }
    }
}
