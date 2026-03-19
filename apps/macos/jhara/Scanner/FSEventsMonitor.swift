// FSEventsMonitor.swift
// Jhara
//
// Real-time filesystem change notifications using the macOS FSEvents API.
//
// After FTSScanner completes an initial scan, the app does not want to
// re-scan everything every time a file changes. FSEventsMonitor watches
// a set of directories at the kernel level and emits coalesced batches
// of changed paths, allowing the app to do targeted re-scans of only the
// directories that actually changed.
//
// Why FSEvents over alternatives:
//   - kqueue: Works per-file, requires an open file descriptor for each
//     monitored path. Watching 10K directories would consume 10K fds.
//   - NSFilePresenter / NSFileCoordinator: Designed for document-based apps
//     sharing files; too heavy and not designed for monitoring large trees.
//   - FSEvents: Watches directory trees at the kernel level, coalesces
//     high-frequency changes, returns paths (not fds), consumes no fd quota.
//     This is how Finder, Spotlight, and Time Machine work internally.
//
// Latency tuning:
//   The coalesced latency parameter controls how long FSEvents waits before
//   delivering a batch. A short latency means more frequent callbacks with
//   smaller batches. A long latency means fewer callbacks with larger batches.
//
//   Foreground app: 30 seconds. This gives the user a responsive experience
//   when they manually delete something or run a build - the treemap updates
//   within half a minute without hammering the disk with re-scans.
//
//   Background automation agent: 600 seconds (10 minutes). The agent does
//   not need real-time accuracy. Batching changes over 10 minutes reduces
//   CPU and disk wake frequency, which matters for battery life.
//
// Thread safety:
//   FSEvents delivers callbacks on its own private thread (the runloop
//   provided at stream creation time). FSEventsMonitor routes all callbacks
//   through an actor to make them safe to consume from Swift concurrency.

import Foundation
import CoreServices

// MARK: - FSEventsMonitor

/// Watches a set of directory paths for changes and delivers batched
/// notifications through an AsyncStream.
///
/// Example (foreground app usage):
/// ```swift
/// let monitor = FSEventsMonitor(latency: 30.0)
/// let watchPaths = [
///     "\(NSHomeDirectory())/Developer",
///     "\(NSHomeDirectory())/.npm",
/// ]
/// await monitor.start(watching: watchPaths)
///
/// for await batch in monitor.changes {
///     for changedPath in batch.changedPaths {
///         // Re-scan just this directory subtree
///     }
/// }
/// ```
public actor FSEventsMonitor {

    // MARK: - Types

    /// A batch of filesystem changes delivered by FSEvents.
    public struct ChangeBatch: Sendable {
        /// The paths that changed. These are directory paths, not individual
        /// file paths. The changed file is somewhere inside the reported
        /// directory, but FSEvents does not tell you which specific file
        /// changed within that directory (unless you use the per-file API,
        /// which we do not need for Jhara's use case).
        public let changedPaths: [String]

        /// The FSEvents event flags for each path. See FSEventStreamEventFlags
        /// for the full list. Common ones: itemCreated, itemRemoved,
        /// itemRenamed, itemModified, mustScanSubDirs.
        public let flags: [FSEventStreamEventFlags]

        /// When FSEvents suggests you must scan subdirectories too.
        public var mustScanSubDirs: Bool {
            flags.contains { ($0 & UInt32(kFSEventStreamEventFlagMustScanSubDirs)) != 0 }
        }

        /// Returns the smallest set of directories that cover all changed paths.
        ///
        /// If the batch contains both `/Users/foo/Developer/jhara` and
        /// `/Users/foo/Developer/jhara/src`, this method returns only
        /// `/Users/foo/Developer/jhara` because scanning it automatically
        /// covers its subdirectories.
        public func minimalCoveringAncestorSet() -> [String] {
            let sorted = changedPaths.sorted()
            var results: [String] = []
            
            for path in sorted {
                if let last = results.last {
                    // Check if current path is a child of the last added path
                    let prefix = last.hasSuffix("/") ? last : last + "/"
                    if path.hasPrefix(prefix) {
                        continue
                    }
                }
                results.append(path)
            }
            return results
        }
    }

    // MARK: - Configuration

    /// How long FSEvents should wait before delivering a batch of changes.
    /// Longer latency = fewer callbacks = better battery life.
    public let latency: TimeInterval

    // MARK: - State

    private var streamRef: FSEventStreamRef?
    private var continuation: AsyncStream<ChangeBatch>.Continuation?
    private var isRunning = false

    // MARK: - Init

    /// - Parameter latency: Coalescing latency in seconds.
    ///   Use 30.0 for the foreground app; 600.0 for the background agent.
    public init(latency: TimeInterval = 30.0) {
        self.latency = latency
    }

    // MARK: - Public interface

    /// An async stream of change batches. Each value is a coalesced batch
    /// of filesystem changes that arrived within one latency window.
    ///
    /// The stream never finishes unless `stop()` is called.
    public var changes: AsyncStream<ChangeBatch> {
        let (stream, cont) = AsyncStream<ChangeBatch>.makeStream()
        self.continuation = cont
        return stream
    }

    /// Begins monitoring the given paths for changes.
    ///
    /// Calling `start` while already running replaces the existing stream
    /// with a new one watching the new paths. The old stream is invalidated.
    ///
    /// - Parameter paths: Absolute directory paths to watch.
    ///   Subdirectories are automatically included.
    public func start(watching paths: [String]) {
        stop() // Clean up any existing stream first.
        guard !paths.isEmpty else { return }

        // The callback we give to FSEvents cannot capture self directly
        // because C callbacks cannot use Swift value types. We bridge through
        // a retained pointer to a callback box.
        let callbackBox = CallbackBox { [weak self] batch in
            guard let self else { return }
            Task {
                await self.deliver(batch: batch)
            }
        }

        // Retain the box for the lifetime of the stream.
        // We release it in `stop()` via `FSEventStreamRelease`.
        let boxPtr = Unmanaged.passRetained(callbackBox).toOpaque()

        var context = FSEventStreamContext(
            version: 0,
            info: boxPtr,
            retain: { ptr in
                guard let ptr else { return nil }
                Unmanaged<CallbackBox>.fromOpaque(ptr).retain()
                return ptr
            },
            release: { ptr in
                guard let ptr else { return }
                Unmanaged<CallbackBox>.fromOpaque(ptr).release()
            },
            copyDescription: nil
        )

        let cfPaths = paths as CFArray
        let flags: FSEventStreamCreateFlags =
            FSEventStreamCreateFlags(kFSEventStreamCreateFlagNoDefer) |
            FSEventStreamCreateFlags(kFSEventStreamCreateFlagFileEvents)

        let stream = FSEventStreamCreate(
            kCFAllocatorDefault,
            { (streamRef, info, numEvents, eventPaths, eventFlags, eventIds) in
                guard let info = info else { return }
                let box = Unmanaged<CallbackBox>.fromOpaque(info).takeUnretainedValue()
                let pathsArray = unsafeBitCast(eventPaths, to: CFArray.self) as? [String] ?? []
                var flags: [FSEventStreamEventFlags] = []
                for i in 0..<numEvents {
                    flags.append(eventFlags[i])
                }
                let batch = FSEventsMonitor.ChangeBatch(
                    changedPaths: Array(pathsArray.prefix(numEvents)),
                    flags: flags
                )
                box.callback(batch)
            },
            &context,
            cfPaths,
            FSEventStreamEventId(kFSEventStreamEventIdSinceNow),
            latency,
            flags
        )

        guard let stream else {
            return
        }

        FSEventStreamScheduleWithRunLoop(
            stream,
            CFRunLoopGetMain(),
            CFRunLoopMode.defaultMode.rawValue
        )
        FSEventStreamStart(stream)

        self.streamRef = stream
        self.isRunning = true
    }

    /// Stops monitoring and invalidates the current change stream.
    public func stop() {
        guard let stream = streamRef else { return }
        FSEventStreamStop(stream)
        FSEventStreamInvalidate(stream)
        FSEventStreamRelease(stream)
        streamRef = nil
        isRunning = false
        continuation?.finish()
        continuation = nil
    }

    // MARK: - Private

    private func deliver(batch: ChangeBatch) {
        continuation?.yield(batch)
    }
}

// MARK: - FSEvents C callback bridge

/// A box that holds the Swift closure we want to call from the C callback.
/// FSEventStreamContext's info pointer points to one of these.
private final class CallbackBox: @unchecked Sendable {
    let callback: (FSEventsMonitor.ChangeBatch) -> Void
    init(callback: @escaping (FSEventsMonitor.ChangeBatch) -> Void) {
        self.callback = callback
    }
}


// MARK: - FSEventStreamEventFlags convenience

extension FSEventStreamEventFlags {
    static let mustScanSubDirs = FSEventStreamEventFlags(kFSEventStreamEventFlagMustScanSubDirs)
    static let itemCreated     = FSEventStreamEventFlags(kFSEventStreamEventFlagItemCreated)
    static let itemRemoved     = FSEventStreamEventFlags(kFSEventStreamEventFlagItemRemoved)
    static let itemRenamed     = FSEventStreamEventFlags(kFSEventStreamEventFlagItemRenamed)
    static let itemModified    = FSEventStreamEventFlags(kFSEventStreamEventFlagItemModified)
}
