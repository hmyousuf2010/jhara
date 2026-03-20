import Foundation
import Observation
import ServiceManagement

@Observable
final class AppState {
    
    // MARK: - Automation State
    
    /// The registration status of the background agent.
    var automationStatus: SMAppService.Status = .notRegistered
    
    /// Whether the background agent is currently performing a task.
    var isAutomationRunning: Bool = false
    
    /// Diagnostic error message if registration fails.
    var registrationError: String? = nil
    
    /// Recent scan history records for rule activity log.
    var recentRuleActivity: [ScanHistoryRecord] = []
    
    // MARK: - Dependencies
    
    private let serviceManager = SMAppServiceManager()
    
    // MARK: - Logic
    
    /// Updates the local status by querying the system.
    @MainActor
    func refreshAutomationStatus() async {
        self.automationStatus = await serviceManager.currentStatus
    }
    
    /// Toggles the background agent registration.
    @MainActor
    func toggleAutomationRegistration() async {
        self.registrationError = nil
        do {
            let current = await serviceManager.currentStatus
            if current == .enabled || current == .requiresApproval {
                try await serviceManager.unregister()
            } else {
                try await serviceManager.register()
                // If it successfully registers but requires approval, 
                // we should keep it "on" in the UI so the user 
                // understands they need to click "Allow" in settings.
            }
            await refreshAutomationStatus()
        } catch {
            self.registrationError = error.localizedDescription
            print("Registration toggle failed: \(error)")
        }
    }
    
    /// Opens the System Settings to the Login Items page.
    func openLoginItemsSettings() {
        serviceManager.openLoginItemsSettings()
    }
}
