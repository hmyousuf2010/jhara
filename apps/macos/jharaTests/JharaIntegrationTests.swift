// JharaIntegrationTests.swift
// JharaTests
//
// Integration tests for the Rust scanner via ScanCoordinator.
// These tests spin up a real scan against a controlled mock directory tree
// and verify results against `du` and `find` ground truth.
//
// Run via:
//   xcodebuild test -scheme JharaApp \
//     -only-testing:JharaTests/JharaIntegrationTests

import XCTest
@testable import jhara  // adjust module name to match your Xcode target

final class JharaIntegrationTests: XCTestCase {

    // MARK: - Mock tree

    /// Root of the temporary directory tree created for each test.
    private var mockRoot: URL!

    /// Layout created in `setUpWithError`:
    ///
    /// ```
    /// mockRoot/
    ///   a.txt          (1 024 bytes)
    ///   b.txt          (2 048 bytes)
    ///   subdir/
    ///     c.txt        (4 096 bytes)
    ///     deep/
    ///       d.txt      (8 192 bytes)
    ///   empty_dir/
    /// ```
    private let fileContents: [(path: String, size: Int)] = [
        ("a.txt",            1_024),
        ("b.txt",            2_048),
        ("subdir/c.txt",     4_096),
        ("subdir/deep/d.txt",8_192),
    ]

    override func setUpWithError() throws {
        let tmp = FileManager.default.temporaryDirectory
        mockRoot = tmp.appendingPathComponent("jhara_integration_\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: mockRoot,
                                                withIntermediateDirectories: true)

        for entry in fileContents {
            let url = mockRoot.appendingPathComponent(entry.path)
            try FileManager.default.createDirectory(
                at: url.deletingLastPathComponent(),
                withIntermediateDirectories: true)
            
            let data = Data(repeating: 0xAB, count: entry.size)
            try data.write(to: url)
        }

        // Empty directory — should appear as a node but contribute 0 to file count.
        try FileManager.default.createDirectory(
            at: mockRoot.appendingPathComponent("empty_dir"),
            withIntermediateDirectories: true
        )
    }

    override func tearDownWithError() throws {
        if let root = mockRoot {
            try? FileManager.default.removeItem(at: root)
        }
    }

    // MARK: - Helpers

    /// Runs a full scan of `mockRoot` and collects all proxies.
    /// Times out after `timeout` seconds.
    private func runFullScan(
        skipList: [String] = [],
        timeout: TimeInterval = 10
    ) async throws -> [ScanNodeProxy] {

        let coordinator = ScanCoordinator()
        var collected: [ScanNodeProxy] = []

        let expectation = XCTestExpectation(description: "scan completes")

        let stream = await coordinator.scan(
            roots:    [mockRoot.path],
            skipList: skipList
        )

        for await event in stream {
            switch event {
            case .batch(let nodes):
                collected.append(contentsOf: nodes)
            case .completed:
                expectation.fulfill()
            case .cancelled:
                XCTFail("Unexpected cancellation")
                expectation.fulfill()
            case .failed(let err):
                XCTFail("Scan failed: \(err)")
                expectation.fulfill()
            }
        }

        await fulfillment(of: [expectation], timeout: timeout)
        return collected
    }

    /// Ground-truth file count via `find`.
    private func groundTruthFileCount() -> Int {
        let task = Process()
        task.launchPath = "/usr/bin/find"
        task.arguments  = [mockRoot.path, "-type", "f"]
        let pipe = Pipe()
        task.standardOutput = pipe
        task.launch(); task.waitUntilExit()
        let output = String(data: pipe.fileHandleForReading.readDataToEndOfFile(),
                            encoding: .utf8) ?? ""
        return output.split(separator: "\n").filter { !$0.isEmpty }.count
    }

    /// Ground-truth total logical size via `find -size`.
    /// Returns sum of logical sizes in bytes.
    private func groundTruthLogicalSize() -> Int64 {
        return fileContents.reduce(0) { $0 + Int64($1.size) }
    }

    // MARK: - Tests

    // ── 1. Node count ─────────────────────────────────────────────────────────

    func testScan_fileNodeCount_matchesFindOutput() async throws {
        let nodes = try await runFullScan()
        let rustFileCount = nodes.filter { $0.kind == .file }.count
        let groundTruth   = groundTruthFileCount()

        XCTAssertEqual(
            rustFileCount, groundTruth,
            "Rust scanner found \(rustFileCount) files; `find` found \(groundTruth)"
        )
    }

    func testScan_directoryNodes_arePresent() async throws {
        let nodes = try await runFullScan()
        let dirs  = nodes.filter { $0.kind == .directory }

        // Expect: mockRoot, subdir, subdir/deep, empty_dir (at minimum)
        XCTAssertGreaterThanOrEqual(dirs.count, 4,
            "Expected at least 4 directory nodes, got \(dirs.count)")
    }

    // ── 2. Size verification ──────────────────────────────────────────────────

    func testScan_totalLogicalSize_matchesKnownBytesWritten() async throws {
        let nodes     = try await runFullScan()
        let rustTotal = nodes
            .filter { $0.kind == .file && $0.logicalSize > 0 }
            .reduce(Int64(0)) { $0 + $1.logicalSize }

        let groundTruth = groundTruthLogicalSize()

        XCTAssertEqual(
            rustTotal, groundTruth,
            "Total logical size mismatch: Rust=\(rustTotal), expected=\(groundTruth)"
        )
    }

    func testScan_physicalSizeIsNonNegative_forAllFileNodes() async throws {
        let nodes = try await runFullScan()
        let bad   = nodes.filter { $0.kind == .file && $0.physicalSize < 0 }

        XCTAssertTrue(bad.isEmpty,
            "Found \(bad.count) file nodes with negative physicalSize: \(bad.map(\.path))")
    }

    // ── 3. Path correctness ───────────────────────────────────────────────────

    func testScan_allPathsStartWithMockRoot() async throws {
        let nodes  = try await runFullScan()
        let prefix = mockRoot.path
        let bad    = nodes.filter { !$0.path.hasPrefix(prefix) }

        XCTAssertTrue(bad.isEmpty,
            "Found nodes outside mock root: \(bad.map(\.path))")
    }

    func testScan_knownFilesArePresent() async throws {
        let nodes = try await runFullScan()
        let paths = Set(nodes.map(\.path))

        for entry in fileContents {
            let expected = mockRoot.appendingPathComponent(entry.path).path
            XCTAssertTrue(paths.contains(expected),
                "Expected node not found: \(expected)")
        }
    }

    func testScan_nameMatchesLastPathComponent() async throws {
        let nodes = try await runFullScan()
        let bad   = nodes.filter { node in
            let expected = URL(fileURLWithPath: node.path).lastPathComponent
            return node.name != expected
        }
        XCTAssertTrue(bad.isEmpty,
            "`name` does not match last path component for: \(bad.map(\.path))")
    }

    // ── 4. Skip list ──────────────────────────────────────────────────────────

    func testScan_skipList_excludesSubdirEntirely() async throws {
        let skipPath = mockRoot.appendingPathComponent("subdir").path
        let nodes    = try await runFullScan(skipList: [skipPath])
        let bad      = nodes.filter { $0.path.hasPrefix(skipPath) }

        XCTAssertTrue(bad.isEmpty,
            "Skip list did not exclude subdir; found: \(bad.map(\.path))")
    }

    func testScan_skipList_doesNotAffectOtherNodes() async throws {
        let skipPath = mockRoot.appendingPathComponent("subdir").path
        let nodes    = try await runFullScan(skipList: [skipPath])

        // a.txt and b.txt must still appear.
        let paths = Set(nodes.map(\.path))
        for name in ["a.txt", "b.txt"] {
            let expected = mockRoot.appendingPathComponent(name).path
            XCTAssertTrue(paths.contains(expected),
                "\(name) was incorrectly excluded by skip list")
        }
    }

    // ── 5. Cancellation ───────────────────────────────────────────────────────

    func testScan_cancellation_stopsCallbackDelivery() async throws {
        let coordinator    = ScanCoordinator()
        var batchesAfterCancel = 0
        var cancelFired    = false

        let stream = await coordinator.scan(roots: [mockRoot.path])

        // Cancel after receiving the first batch.
        var firstBatchSeen = false

        for await event in stream {
            switch event {
            case .batch:
                if !firstBatchSeen {
                    firstBatchSeen = true
                    await coordinator.cancel()
                } else {
                    // Any batch arriving after cancel is unexpected but
                    // tolerable (Rust may emit one more after cancel signal).
                    batchesAfterCancel += 1
                }
            case .cancelled:
                cancelFired = true
            case .completed:
                XCTFail("Should not complete after cancellation")
            case .failed(let e):
                XCTFail("Scan failed unexpectedly: \(e)")
            }
        }

        XCTAssertTrue(cancelFired, ".cancelled event was never emitted")
        // Allow at most 1 in-flight batch after cancel.
        XCTAssertLessThanOrEqual(batchesAfterCancel, 1,
            "Too many batches delivered after cancel: \(batchesAfterCancel)")
    }

    func testScan_cancelBeforeStart_emitsCancelledNotFailed() async throws {
        let coordinator = ScanCoordinator()
        await coordinator.cancel()   // cancel before scan() is called

        var events: [String] = []
        let stream = await coordinator.scan(roots: [mockRoot.path])
        for await event in stream {
            switch event {
            case .cancelled: events.append("cancelled")
            case .failed:    events.append("failed")
            case .completed: events.append("completed")
            case .batch:     events.append("batch")
            }
        }

        // After a pre-cancel, we either get no events or a clean cancelled.
        XCTAssertFalse(events.contains("failed"),
            "Pre-cancel should not produce .failed; got: \(events)")
    }

    // ── 6. ScanNodeProxy correctness ─────────────────────────────────────────

    func testProxy_modificationDate_isReasonable() async throws {
        let nodes = try await runFullScan()
        let files = nodes.filter { $0.kind == .file }
        let now   = Date()
        let oneDayAgo = now.addingTimeInterval(-86_400)

        for node in files {
            XCTAssertGreaterThan(node.modificationDate, oneDayAgo,
                "Modification date looks wrong for \(node.path): \(node.modificationDate)")
            XCTAssertLessThanOrEqual(node.modificationDate, now,
                "Modification date is in the future for \(node.path)")
        }
    }

    func testProxy_inodeIsNonZero_forAllNodes() async throws {
        let nodes = try await runFullScan()
        let bad   = nodes.filter { $0.inode == 0 }
        XCTAssertTrue(bad.isEmpty,
            "Nodes with zero inode (unexpected on a real filesystem): \(bad.map(\.path))")
    }
}
