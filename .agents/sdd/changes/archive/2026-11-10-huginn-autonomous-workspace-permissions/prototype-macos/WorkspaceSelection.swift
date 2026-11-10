import Foundation
import AppKit

@discardableResult
func configureWorkspacePanel(_ panel: NSOpenPanel, identifier: NSUserInterfaceItemIdentifier, fixtureRoot: URL) -> Bool {
    var isDirectory: ObjCBool = false
    guard FileManager.default.fileExists(atPath: fixtureRoot.path, isDirectory: &isDirectory), isDirectory.boolValue else {
        return false
    }

    panel.identifier = identifier
    panel.directoryURL = fixtureRoot

    return panel.identifier == identifier
        && panel.directoryURL?.standardizedFileURL == fixtureRoot.standardizedFileURL
}

struct WorkspaceSelectionValidation {
    let accepted: Bool
    let diagnostic: String
}

struct WorkspaceSelectionDecision {
    let shouldLaunchHelper: Bool
    let userMessage: String
}

func workspaceSelectionDecision(_ validation: WorkspaceSelectionValidation) -> WorkspaceSelectionDecision {
    WorkspaceSelectionDecision(
        shouldLaunchHelper: validation.accepted,
        userMessage: validation.accepted ? validation.diagnostic : "BLOCKED: \(validation.diagnostic)"
    )
}

func validateWorkspaceSelection(expected: URL, selected: URL, fixtureRoot: URL) -> WorkspaceSelectionValidation {
    // Resolve existing aliases before comparison. Component-aware containment prevents
    // prefix collisions such as /fixture-elsewhere being treated as inside /fixture.
    let canonicalRoot = fixtureRoot.resolvingSymlinksInPath().standardizedFileURL
    let canonicalExpected = expected.resolvingSymlinksInPath().standardizedFileURL
    let canonicalSelected = selected.resolvingSymlinksInPath().standardizedFileURL
    func isWithinFixture(_ url: URL) -> Bool {
        url.path == canonicalRoot.path || url.path.hasPrefix(canonicalRoot.path.hasSuffix("/") ? canonicalRoot.path : canonicalRoot.path + "/")
    }
    var isDirectory: ObjCBool = false
    let selectedExistsAsDirectory = FileManager.default.fileExists(atPath: selected.path, isDirectory: &isDirectory) && isDirectory.boolValue
    let expectedIsGeneratedWorkspace = canonicalExpected.lastPathComponent == "chosen-workspace"
        && isWithinFixture(canonicalExpected)
        && FileManager.default.fileExists(atPath: canonicalRoot.appendingPathComponent(".prototype-fixture").path)
    let accepted = expectedIsGeneratedWorkspace
        && selectedExistsAsDirectory
        && isWithinFixture(canonicalSelected)
        && canonicalSelected == canonicalExpected
    let safeExpected = isWithinFixture(canonicalExpected) ? canonicalExpected.path : "<outside temporary fixture>"
    let safeSelected = isWithinFixture(canonicalSelected) ? canonicalSelected.path : "<outside temporary fixture>"
    let basename = selected.lastPathComponent
    return WorkspaceSelectionValidation(
        accepted: accepted,
        diagnostic: "workspace_selection expected=\(safeExpected) received=\(safeSelected) selected_basename=\(basename) result=\(accepted ? "accepted" : "blocked")"
    )
}
