import XCTest

// MARK: - Mock Git Checker

final class MockGitChecker: GitChecking {
    enum Behavior { case clean, dirty, notARepo, fail }
    var behavior: Behavior = .clean

    func status(at url: URL) async -> GitStatusResult {
        switch behavior {
        case .clean:    return .clean
        case .dirty:    return .dirty(statusOutput: " M Sources/Main.swift\n?? temp.txt\n")
        case .notARepo: return .notAGitRepo
        case .fail:     return .checkFailed(URLError(.badURL))
        }
    }
}

// MARK: - Mock FileManager Subclass

final class MockFileManager: FileManager {
    var trashedURLs: [URL] = []
    var shouldFail = false
    var failError: Error = CocoaError(.fileWriteNoPermission)

    override func trashItem(at url: URL, resultingItemURL outResultingURL: AutoreleasingUnsafeMutablePointer<NSURL?>?) throws {
        if shouldFail { throw failError }
        trashedURLs.append(url)
    }
}

// MARK: - Test Helpers

private func makeScanItem(
    path: String,
    tier: DeletionTier,
    sizeOnDisk: Int64 = 1_024,
    fileCount: Int = 10,
    isInsideGitProject: Bool = false,
    isCloudSyncedPath: Bool = false
) -> ScanItem {
    ScanItem(
        url: URL(fileURLWithPath: path),
        tier: tier,
        sizeOnDisk: sizeOnDisk,
        fileCount: fileCount,
        isInsideGitProject: isInsideGitProject,
        isCloudSyncedPath: isCloudSyncedPath
    )
}

// MARK: - DeletionPlanTests

final class DeletionPlanTests: XCTestCase {

    // MARK: Stage ordering

    func testStagesAreOrderedLowToHigh() async {
        let items = [
            makeScanItem(path: "/project/target", tier: .heavyCompilation),
            makeScanItem(path: "/project/node_modules/.cache", tier: .pureCaches),
            makeScanItem(path: "/project/dist", tier: .buildOutputs),
        ]
        let plan = await DeletionPlan.build(from: items)

        XCTAssertEqual(plan.stages.count, 3)
        XCTAssertEqual(plan.stages[0].tier, .pureCaches)
        XCTAssertEqual(plan.stages[1].tier, .buildOutputs)
        XCTAssertEqual(plan.stages[2].tier, .heavyCompilation)
    }

    func testEmptyInputProducesEmptyPlan() async {
        let plan = await DeletionPlan.build(from: [])
        XCTAssertTrue(plan.isEmpty)
        XCTAssertEqual(plan.stageCount, 0)
    }

    // MARK: Size aggregation

    func testTotalSizeAggregatesAcrossStages() async {
        let items = [
            makeScanItem(path: "/a", tier: .pureCaches, sizeOnDisk: 500),
            makeScanItem(path: "/b", tier: .buildOutputs, sizeOnDisk: 1_500),
        ]
        let plan = await DeletionPlan.build(from: items)
        XCTAssertEqual(plan.totalSizeOnDisk, 2_000)
    }

    // MARK: Git safety — dirty repo

    func testDirtyGitRepoExcludesItem() async {
        let checker = MockGitChecker()
        checker.behavior = .dirty

        let item = makeScanItem(
            path: "/myproject/node_modules",
            tier: .dependencyDirs,
            isInsideGitProject: true
        )
        let plan = await DeletionPlan.build(from: [item], gitChecker: checker)

        XCTAssertTrue(plan.isEmpty, "Dirty-repo item should be excluded from plan")
        XCTAssertEqual(plan.excludedItems.count, 1)
        XCTAssertTrue(plan.excludedItems[0].reason.contains("uncommitted changes"))
    }

    func testCleanGitRepoPassesThrough() async {
        let checker = MockGitChecker()
        checker.behavior = .clean

        let item = makeScanItem(
            path: "/myproject/node_modules",
            tier: .dependencyDirs,
            isInsideGitProject: true
        )
        let plan = await DeletionPlan.build(from: [item], gitChecker: checker)
        XCTAssertFalse(plan.isEmpty)
        XCTAssertEqual(plan.excludedItems.count, 0)
    }

    func testGitCheckFailureExcludesItem() async {
        let checker = MockGitChecker()
        checker.behavior = .fail

        let item = makeScanItem(path: "/project/.venv", tier: .dependencyDirs, isInsideGitProject: true)
        let plan = await DeletionPlan.build(from: [item], gitChecker: checker)
        XCTAssertTrue(plan.isEmpty)
        XCTAssertTrue(plan.excludedItems[0].reason.contains("failed"))
    }

    // MARK: Cloud-sync guard

    func testCloudSyncedItemIsExcluded() async {
        let item = makeScanItem(path: "/Users/me/Library/Mobile Documents/project/dist",
                                tier: .buildOutputs,
                                isCloudSyncedPath: true)
        let plan = await DeletionPlan.build(from: [item])
        XCTAssertTrue(plan.isEmpty)
        XCTAssertEqual(plan.excludedItems.count, 1)
        XCTAssertTrue(plan.excludedItems[0].reason.contains("cloud-synced"))
    }

    // MARK: Pre-deletion summary

    func testPreDeletionSummaryContainsKeyInfo() async {
        let items = [
            makeScanItem(path: "/proj/node_modules/.cache", tier: .pureCaches, sizeOnDisk: 1024),
            makeScanItem(path: "/proj/dist", tier: .buildOutputs, sizeOnDisk: 2048),
        ]
        let plan = await DeletionPlan.build(from: items)
        let summary = plan.preDeletionSummary()

        XCTAssertTrue(summary.contains("Level 1"))
        XCTAssertTrue(summary.contains("Level 2"))
        XCTAssertTrue(summary.contains("Pure Caches"))
        XCTAssertTrue(summary.contains("Build Outputs"))
    }
}

// MARK: - TrashCoordinatorTests

final class TrashCoordinatorTests: XCTestCase {

    // MARK: Dry run

    func testDryRunDoesNotMutateFilesystem() async {
        var config = TrashCoordinator.Configuration()
        config.dryRun = true

        let coordinator = TrashCoordinator(configuration: config, confirmationHandler: { _ in true })
        let items = [
            makeScanItem(path: "/fake/dist", tier: .buildOutputs, sizeOnDisk: 512),
        ]
        let plan = DeletionPlan.buildSynchronously(from: items)
        let summary = await coordinator.execute(plan: plan)

        XCTAssertEqual(summary.trashedCount, 1)
        XCTAssertEqual(summary.totalBytesReclaimed, 512)
        XCTAssertFalse(summary.wasInterrupted)
    }

    // MARK: Level 5 confirmation — declined

    func testStatefulItemSkippedWhenConfirmationDeclined() async {
        var config = TrashCoordinator.Configuration()
        config.dryRun = true

        let coordinator = TrashCoordinator(
            configuration: config,
            confirmationHandler: { _ in false } // always decline
        )
        let items = [makeScanItem(path: "/fake/conda-env", tier: .statefulItems)]
        let plan = DeletionPlan.buildSynchronously(from: items)
        let summary = await coordinator.execute(plan: plan)

        XCTAssertEqual(summary.skippedCount, 1)
        XCTAssertEqual(summary.trashedCount, 0)
    }

    func testStatefulItemTrashedWhenConfirmationAccepted() async {
        var config = TrashCoordinator.Configuration()
        config.dryRun = true

        let coordinator = TrashCoordinator(
            configuration: config,
            confirmationHandler: { _ in true } // always accept
        )
        let items = [makeScanItem(path: "/fake/conda-env", tier: .statefulItems, sizeOnDisk: 4096)]
        let plan = DeletionPlan.buildSynchronously(from: items)
        let summary = await coordinator.execute(plan: plan)

        XCTAssertEqual(summary.trashedCount, 1)
        XCTAssertEqual(summary.totalBytesReclaimed, 4096)
    }

    // MARK: Interrupted deletion

    func testInterruptedDeletionSetsFlagAndRecordsPartialResults() async {
        var config = TrashCoordinator.Configuration()
        config.dryRun = true

        let coordinator = TrashCoordinator(configuration: config, confirmationHandler: { _ in true })
        let items = (0..<10).map {
            makeScanItem(path: "/fake/item\($0)", tier: .buildOutputs, sizeOnDisk: 100)
        }
        let plan = DeletionPlan.buildSynchronously(from: items)

        let task = Task {
            await coordinator.execute(plan: plan)
        }

        // Cancel almost immediately.
        try? await Task.sleep(nanoseconds: 1_000_000) // 1 ms
        task.cancel()

        let summary = await task.value

        // Must be marked as interrupted and have processed fewer than all items
        // (exact count is non-deterministic; just ensure the flag is set).
        XCTAssertTrue(summary.wasInterrupted)
        XCTAssertLessThan(summary.trashedCount + summary.skippedCount + summary.failedCount, items.count)
    }

    // MARK: Partial completion recovery

    func testResumeCompletesRemainingItemsAfterInterruption() async {
        var config = TrashCoordinator.Configuration()
        config.dryRun = true

        let coordinator = TrashCoordinator(configuration: config, confirmationHandler: { _ in true })

        // 10 items; interrupt after ~1 ms to get a partial first run.
        let items = (0..<10).map {
            makeScanItem(path: "/fake/item\($0)", tier: .buildOutputs, sizeOnDisk: 100)
        }
        let plan = DeletionPlan.buildSynchronously(from: items)

        let firstTask = Task { await coordinator.execute(plan: plan) }
        try? await Task.sleep(nanoseconds: 1_000_000)
        firstTask.cancel()
        let firstSummary = await firstTask.value

        XCTAssertTrue(firstSummary.wasInterrupted)

        // Resume
        let finalSummary = await coordinator.resume(from: plan, previousSummary: firstSummary)

        // All items must be accounted for after resume.
        let total = finalSummary.trashedCount + finalSummary.skippedCount + finalSummary.failedCount
        XCTAssertEqual(total, items.count, "All items should be accounted for after resume")
        XCTAssertFalse(finalSummary.wasInterrupted)
    }

    // MARK: Progress reporting

    func testProgressHandlerCalledForEachItem() async {
        var config = TrashCoordinator.Configuration()
        config.dryRun = true

        let coordinator = TrashCoordinator(configuration: config, confirmationHandler: { _ in true })
        let count = 5
        let items = (0..<count).map { makeScanItem(path: "/fake/item\($0)", tier: .buildOutputs) }
        let plan = DeletionPlan.buildSynchronously(from: items)

        var progressCallCount = 0
        _ = await coordinator.execute(plan: plan) { _ in
            progressCallCount += 1
        }

        XCTAssertEqual(progressCallCount, count)
    }

    func testProgressFractionReachesOne() async {
        var config = TrashCoordinator.Configuration()
        config.dryRun = true

        let coordinator = TrashCoordinator(configuration: config, confirmationHandler: { _ in true })
        let items = (0..<4).map { makeScanItem(path: "/fake/b\($0)", tier: .buildOutputs) }
        let plan = DeletionPlan.buildSynchronously(from: items)

        var lastFraction: Double = 0
        _ = await coordinator.execute(plan: plan) { progress in
            lastFraction = progress.fractionCompleted
        }

        XCTAssertEqual(lastFraction, 1.0, accuracy: 0.001)
    }

    // MARK: Post-deletion summary content

    func testPostDeletionSummaryContainsExpectedFields() async {
        var config = TrashCoordinator.Configuration()
        config.dryRun = true

        let coordinator = TrashCoordinator(configuration: config, confirmationHandler: { _ in true })
        let items = [makeScanItem(path: "/fake/dist", tier: .buildOutputs, sizeOnDisk: 2048)]
        let plan = DeletionPlan.buildSynchronously(from: items)
        let summary = await coordinator.execute(plan: plan)

        let text = summary.postDeletionSummary()
        XCTAssertTrue(text.contains("Trashed"))
        XCTAssertTrue(text.contains("Reclaimed"))
        XCTAssertTrue(text.contains("✓  Run completed normally."))
    }
}
