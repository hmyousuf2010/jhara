// ScanCoordinator.swift
// jhara — macOS
//
// Swift Actor that owns the full lifecycle of a Rust scan session.
// This is the ONLY file in the Swift layer that touches raw FFI types.
// Everything above this layer works with `ScanNodeProxy` — a safe,
// Swift-native value type.
//
// NOTE: jhara_scan_start / jhara_scan_cancel / jhara_scan_free / ScanNodeC /
// ScanNodeBatchC are declared in jhara_core.h, imported via the
// Objective-C Bridging Header (JharaApp-Bridging-Header.h).
// These symbols will be unresolved until the Rust static library
// (libjhara_universal.a) is built and linked in Xcode.
// Run `scripts/build_universal.sh` once before building the app.

import Foundation

// MARK: - ScanNodeProxy

/// Swift-native, memory-safe mirror of `ScanNodeC`.
///
/// Created once per FFI node inside the batch callback, immediately copying
/// the arena-owned C strings into Swift `String` values.  After this point
/// no raw pointers are retained anywhere in the Swift layer.
public struct ScanNodeProxy: Sendable {
    public let path:              String
    public let name:              String
    public let inode:             UInt64
    public let physicalSize:      Int64
    public let logicalSize:       Int64
    public let modificationDate:  Date
    public let linkCount:         UInt16
    public let kind:              NodeKind

    public enum NodeKind: UInt8, Sendable {
        case unknown   = 0
        case file      = 1
        case directory = 2
        case symlink   = 3
    }

    /// Initialises from a raw `ScanNodeC`.
    init(raw: ScanNodeC) {
        self.path = raw.path.map { String(cString: $0) } ?? ""
        self.name = raw.name.map { String(cString: $0) } ?? ""
        self.inode        = raw.inode
        self.physicalSize = raw.physical_size
        self.logicalSize  = raw.logical_size
        self.linkCount    = raw.link_count
        self.kind         = NodeKind(rawValue: raw.kind) ?? .unknown
        let secs  = TimeInterval(raw.modification_secs)
        let nanos = TimeInterval(raw.modification_nanos) / 1_000_000_000
        self.modificationDate = Date(timeIntervalSince1970: secs + nanos)
    }

    /// Stub initialiser used when FFI library is not yet linked (development).
    public init(
        path: String,
        name: String,
        inode: UInt64 = 0,
        physicalSize: Int64 = 0,
        logicalSize: Int64 = 0,
        modificationDate: Date = .now,
        linkCount: UInt16 = 1,
        kind: NodeKind = .file
    ) {
        self.path             = path
        self.name             = name
        self.inode            = inode
        self.physicalSize     = physicalSize
        self.logicalSize      = logicalSize
        self.modificationDate = modificationDate
        self.linkCount        = linkCount
        self.kind             = kind
    }
}

// MARK: - ScanEvent

/// Events emitted by `ScanCoordinator` to its caller.
public enum ScanEvent: Sendable {
    case batch([ScanNodeProxy])
    case completed
    case cancelled
    case failed(ScanCoordinatorError)
}

public enum ScanCoordinatorError: Error, Sendable {
    case invalidRoots
    case rustInitFailed
}

// MARK: - Thread-safe batch buffer
// Kept outside the actor so nonisolated C callbacks can safely mutate it.

private final class BatchBuffer: @unchecked Sendable {
    private let lock = NSLock()
    private var batches: [[ScanNodeProxy]] = []

    func append(_ batch: [ScanNodeProxy]) {
        lock.withLock { batches.append(batch) }
    }

    func drain() -> [[ScanNodeProxy]] {
        lock.withLock {
            let current = batches
            batches.removeAll(keepingCapacity: true)
            return current
        }
    }
}

// MARK: - ScanCoordinator

/// Actor that manages one Rust scan session end-to-end.
public actor ScanCoordinator {

    // MARK: Private state

    private nonisolated(unsafe) var handle: OpaquePointer?   // *mut JharaScanHandle
    /// Exposed for AppState to call jhara_core_projects_results_json after scan completes.
    /// Only valid between scan completion and jhara_core_scan_free.
    nonisolated(unsafe) var currentHandle: OpaquePointer? { handle }
    private var isCancelled: Bool = false
    private var continuation: AsyncStream<ScanEvent>.Continuation?

    // Kept outside actor isolation so the C callback can write to it safely.
    private let buffer = BatchBuffer()

    // MARK: - Public API

    public func scan(
        roots:    [String],
        skipList: [String] = []
    ) -> AsyncStream<ScanEvent> {
        AsyncStream { continuation in
            self.continuation = continuation
            self.startRustScan(roots: roots, skipList: skipList)
        }
    }

    public func cancel() {
        guard !isCancelled else { return }
        isCancelled = true
        if let h = handle { jhara_core_scan_cancel(h) }
        continuation?.yield(.cancelled)
        continuation?.finish()
        freeHandle()
    }

    deinit { freeHandle() }

    // MARK: - Private helpers

    private func startRustScan(roots: [String], skipList: [String]) {
        guard !roots.isEmpty else {
            continuation?.yield(.failed(.invalidRoots))
            continuation?.finish()
            return
        }

        withArrayOfCStrings(roots) { rootsPtr in
        withArrayOfCStrings(skipList) { skipPtr in
            let ctx = Unmanaged.passRetained(self).toOpaque()
            let h = jhara_core_scan_start(
                rootsPtr, roots.count,
                skipPtr,  skipList.count,
                scanBatchCallback,
                ctx
            )
            if let h {
                self.handle = h
            } else {
                Unmanaged<ScanCoordinator>.fromOpaque(ctx).release()
                continuation?.yield(.failed(.rustInitFailed))
                continuation?.finish()
            }
        }}
    }

    /// Called from the C callback (off-actor, potentially concurrent).
    /// `nonisolated` + `BatchBuffer` (has its own NSLock) — no actor hop needed.
    nonisolated func unsafeEnqueue(_ batch: [ScanNodeProxy]) {
        buffer.append(batch)
        Task { await self.flushPending() }
    }

    nonisolated func unsafeSignalDone(ctx: UnsafeMutableRawPointer?) {
        if let ctx {
            Unmanaged<ScanCoordinator>.fromOpaque(ctx).release()
        }
        Task { await self.finishScan() }
    }

    private func flushPending() {
        for batch in buffer.drain() where !batch.isEmpty {
            continuation?.yield(.batch(batch))
        }
    }

    private func finishScan() {
        guard !isCancelled else { return }
        continuation?.yield(.completed)
        continuation?.finish()
        freeHandle()
    }

    private func freeHandle() {
        if let h = handle { jhara_core_scan_free(h); handle = nil }
    }
}

// MARK: - C callback (file-scope, @convention(c))

private let scanBatchCallback: @convention(c) (ScanNodeBatchC, UnsafeMutableRawPointer?) -> Void = {
    batch, ctx in
    guard let ctx else { return }
    let coordinator = Unmanaged<ScanCoordinator>.fromOpaque(ctx).takeUnretainedValue()
    if batch.count == 0 {
        coordinator.unsafeSignalDone(ctx: ctx)
        return
    }
    guard let nodePtr = batch.nodes else { return }
    let proxies = (0..<batch.count).map { i in ScanNodeProxy(raw: nodePtr[i]) }
    coordinator.unsafeEnqueue(proxies)
}

// MARK: - Utility

private func withArrayOfCStrings<R>(
    _ strings: [String],
    body: (UnsafePointer<UnsafePointer<CChar>?>) -> R
) -> R {
    var cStrings = strings.map { $0.withCString { strdup($0) } }
    defer { cStrings.forEach { free($0) } }
    return cStrings.withUnsafeMutableBufferPointer { buf in
        buf.withMemoryRebound(to: UnsafePointer<CChar>?.self) { reboundBuf in
            body(reboundBuf.baseAddress!)
        }
    }
}
