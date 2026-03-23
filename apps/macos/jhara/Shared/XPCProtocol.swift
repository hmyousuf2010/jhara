import Foundation

// MARK: - XPC Interface Name

public let kJharaXPCServiceName = "com.hmyousuf.jhara.automation.xpc"

// MARK: - XPC Messages (Codable DTOs)

/// Sent from main app → agent to trigger a cleanup run.
public struct XPCCleanNowRequest: Codable, Sendable {
    public let ruleId: UUID
    public let overrideTierLimit: Int? 
    public let dryRun: Bool

    public init(ruleId: UUID, overrideTierLimit: Int? = nil, dryRun: Bool = false) {
        self.ruleId = ruleId
        self.overrideTierLimit = overrideTierLimit
        self.dryRun = dryRun
    }
}

/// Response carrying a serialised scan result back to the requesting side.
public struct XPCCleanNowResponse: Codable, Sendable {
    public let scanHistoryId: UUID
    public let totalBytesReclaimed: Int64
    public let itemsTrashed: Int
    public let itemsFailed: Int
    public let wasInterrupted: Bool
    public let errorMessage: String?

    public init(
        scanHistoryId: UUID,
        totalBytesReclaimed: Int64,
        itemsTrashed: Int,
        itemsFailed: Int,
        wasInterrupted: Bool,
        errorMessage: String? = nil
    ) {
        self.scanHistoryId = scanHistoryId
        self.totalBytesReclaimed = totalBytesReclaimed
        self.itemsTrashed = itemsTrashed
        self.itemsFailed = itemsFailed
        self.wasInterrupted = wasInterrupted
        self.errorMessage = errorMessage
    }
}

// MARK: - XPC Protocol

@objc public protocol JharaXPCProtocol {
    func cleanNow(requestData: Data, reply: @escaping @Sendable (Data?, Error?) -> Void)
    func ping(reply: @escaping @Sendable (String) -> Void)
}
