import Foundation
import ServiceManagement

/// Manages the lifecycle of the `JharaAutomationAgent` login-item agent
/// via `SMAppService` (macOS 13+).
///
/// Appears in: System Settings › General › Login Items & Extensions
public actor SMAppServiceManager {
    
    // MARK: - Constants
    
    /// The bundle identifier of the jhara-agent app.
    private static let agentIdentifier = "com.hmyousuf.jhara.agent"
    
    // MARK: - Properties
    
    private let service: SMAppService
    
    // MARK: - Init
    
    public init() {
        self.service = SMAppService.loginItem(identifier: Self.agentIdentifier)
    }
    
    /// Initialiser for testing — accepts an injected service.
    public init(service: SMAppService) {
        self.service = service
    }
    
    // MARK: - Registration Actions
    
    /// Returns the current raw registration status.
    public var currentStatus: SMAppService.Status {
        service.status
    }
    
    /// Registers the agent as a Login Item.
    public func register() throws {
        try service.register()
    }
    
    /// Unregisters (removes) the agent.
    public func unregister() throws {
        try service.unregister()
    }
    
    /// Opens the "Login Items & Extensions" panel in System Settings.
    /// This is where the user must manually "Allow" the background task if required.
    public nonisolated func openLoginItemsSettings() {
        SMAppService.openLoginItemSettings()
    }
}
