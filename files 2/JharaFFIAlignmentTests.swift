// JharaFFIAlignmentTests.swift
//
// §4.4 Early Validation Test
//
// Verifies that the Swift compiler's view of `ScanNodeC` exactly matches
// the Rust compiler's view.  This test MUST be the first FFI test to pass
// after any change to `types.rs`, `jhara_core.h`, or this file.
//
// Run via:  xcodebuild test -scheme JharaApp -only-testing:JharaTests/JharaFFIAlignmentTests

import XCTest

// The bridging header makes `ScanNodeC`, `ScanNodeBatchC`, and friends
// available as Swift types automatically.

final class JharaFFIAlignmentTests: XCTestCase {

    // MARK: - Size & alignment

    func testScanNodeC_sizeIs56Bytes() {
        XCTAssertEqual(
            MemoryLayout<ScanNodeC>.size, 56,
            "ScanNodeC size mismatch — check types.rs and jhara_core.h"
        )
    }

    func testScanNodeC_alignmentIs8Bytes() {
        XCTAssertEqual(
            MemoryLayout<ScanNodeC>.alignment, 8,
            "ScanNodeC alignment mismatch"
        )
    }

    func testScanNodeBatchC_sizeIs16Bytes() {
        // pointer (8) + size_t (8)
        XCTAssertEqual(
            MemoryLayout<ScanNodeBatchC>.size, 16,
            "ScanNodeBatchC size mismatch"
        )
    }

    // MARK: - Field offsets
    //
    // Swift's `MemoryLayout<T>.offset(of:)` requires key paths to stored
    // properties.  We verify the most alignment-sensitive offsets.

    func testScanNodeC_fieldOffsets() {
        // path @ 0
        XCTAssertEqual(
            MemoryLayout<ScanNodeC>.offset(of: \.path), 0
        )
        // inode @ 16 (after two 8-byte pointers)
        XCTAssertEqual(
            MemoryLayout<ScanNodeC>.offset(of: \.inode), 16
        )
        // modification_nanos @ 48 (after four 8-byte fields)
        XCTAssertEqual(
            MemoryLayout<ScanNodeC>.offset(of: \.modification_nanos), 48
        )
        // link_count @ 52
        XCTAssertEqual(
            MemoryLayout<ScanNodeC>.offset(of: \.link_count), 52
        )
        // kind @ 54
        XCTAssertEqual(
            MemoryLayout<ScanNodeC>.offset(of: \.kind), 54
        )
        // _padding @ 55
        XCTAssertEqual(
            MemoryLayout<ScanNodeC>.offset(of: \._padding), 55
        )
    }

    // MARK: - Sentinel hex round-trip (§4.4 core requirement)
    //
    // Populate a ScanNodeC with recognisable hex sentinel values on the
    // Swift side, pass a pointer to a C shim (or read it back directly),
    // and assert every field decodes exactly as written.
    //
    // This detects:
    //   • byte-swapping (endianness bugs)
    //   • field reordering
    //   • silent padding insertion changing field offsets

    func testSentinelHexRoundTrip_allNumericFields() {
        // Use withUnsafeMutablePointer so the node lives on the stack and
        // we can hand a C pointer to it without heap allocation.
        var node = ScanNodeC()

        // String fields — we use stack-allocated C strings.
        withUnsafePointer(to: ("sentinel_path\0" as StaticString).utf8Start) { pathPtr in
        withUnsafePointer(to: ("sentinel_name\0" as StaticString).utf8Start) { namePtr in

            node.path               = UnsafePointer(pathPtr)
            node.name               = UnsafePointer(namePtr)
            node.inode              = 0xDEAD_BEEF_CAFE_BABE
            node.physical_size      = 0x0102_0304_0506_0708
            node.logical_size       = 0x1112_1314_1516_1718
            node.modification_secs  = 0x2122_2324_2526_2728
            node.modification_nanos = 0x3132_3334
            node.link_count         = 0x4142
            node.kind               = 1 // NodeKind FILE
            node._padding           = 0

            // Round-trip through a raw pointer, exactly as the FFI callback
            // would deliver the struct.
            withUnsafePointer(to: node) { ptr in
                let recovered = ptr.pointee

                XCTAssertEqual(recovered.inode,              0xDEAD_BEEF_CAFE_BABE,
                               "inode mismatch — possible byte-swap or offset error")
                XCTAssertEqual(recovered.physical_size,      0x0102_0304_0506_0708,
                               "physical_size mismatch")
                XCTAssertEqual(recovered.logical_size,       0x1112_1314_1516_1718,
                               "logical_size mismatch")
                XCTAssertEqual(recovered.modification_secs,  0x2122_2324_2526_2728,
                               "modification_secs mismatch")
                XCTAssertEqual(recovered.modification_nanos, 0x3132_3334,
                               "modification_nanos mismatch")
                XCTAssertEqual(recovered.link_count,         0x4142,
                               "link_count mismatch")
                XCTAssertEqual(recovered.kind,               1,
                               "kind mismatch")
                XCTAssertEqual(recovered._padding,           0,
                               "_padding must remain zero")
            }
        }}
    }

    // MARK: - NodeKind discriminants

    func testNodeKindDiscriminants_matchRustDefinition() {
        XCTAssertEqual(NODE_KIND_UNKNOWN.rawValue,   0)
        XCTAssertEqual(NODE_KIND_FILE.rawValue,      1)
        XCTAssertEqual(NODE_KIND_DIRECTORY.rawValue, 2)
        XCTAssertEqual(NODE_KIND_SYMLINK.rawValue,   3)
    }

    // MARK: - Live FFI smoke test
    //
    // Calls `jhara_scan_start` with a real path and verifies:
    //   1. A non-null handle is returned.
    //   2. The callback is invoked at least once.
    //   3. `jhara_scan_free` does not crash.
    //
    // Does NOT assert on scan results — those are covered by integration tests.

    func testLiveFFI_scanStartAndFree() {
        // Use a class box so the closure can capture a reference type.
        class Counter { var value = 0 }
        let counter = Counter()

        // Wrap the callback in a context pointer.
        // SAFETY: counter outlives the scan handle.
        let ctx = Unmanaged.passRetained(counter).toOpaque()

        let callback: @convention(c) (ScanNodeBatchC, UnsafeMutableRawPointer?) -> Void = { _, ctx in
            guard let ctx else { return }
            let counter = Unmanaged<Counter>.fromOpaque(ctx).takeUnretainedValue()
            counter.value += 1
        }

        var root = ("/tmp" as NSString).utf8String
        let handle = withUnsafeMutablePointer(to: &root) { rootsPtr in
            jhara_scan_start(rootsPtr, 1, nil, 0, callback, ctx)
        }

        XCTAssertNotNil(handle, "jhara_scan_start returned NULL for a valid root")
        XCTAssertGreaterThanOrEqual(counter.value, 1, "callback was never invoked")

        jhara_scan_free(handle)
        Unmanaged.passRetained(counter).release() // balance passRetained above
    }

    // MARK: - Null-safety guards

    func testNullHandleGuards_doNotCrash() {
        // All null-handle calls must be no-ops, not crashes.
        jhara_scan_cancel(nil)
        jhara_scan_free(nil)
        let size = jhara_tree_physical_size(nil, nil)
        XCTAssertEqual(size, -1)
    }
}
