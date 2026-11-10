import Foundation
import AppKit

func require(_ condition: @autoclosure () -> Bool, _ message: String) {
    guard condition() else {
        fputs("FAIL: \(message)\n", stderr)
        exit(1)
    }
}

@main
enum WorkspaceSelectionTests {
    static func main() throws {
        let manager = FileManager.default
        let root = manager.temporaryDirectory.appendingPathComponent("workspace-selection-test-\(UUID().uuidString)", isDirectory: true)
        defer { try? manager.removeItem(at: root) }
        let expected = root.appendingPathComponent("chosen-workspace", isDirectory: true)
        let different = root.appendingPathComponent("sibling-workspace", isDirectory: true)
        try manager.createDirectory(at: expected, withIntermediateDirectories: true)
        try manager.createDirectory(at: different, withIntermediateDirectories: true)
        try Data("fixture\n".utf8).write(to: root.appendingPathComponent(".prototype-fixture"))

        let panel = NSOpenPanel()
        let runIdentifier = NSUserInterfaceItemIdentifier("prototype-run-\(UUID().uuidString)")
        require(configureWorkspacePanel(panel, identifier: runIdentifier, fixtureRoot: root), "panel configuration must succeed without presenting UI")
        require(panel.identifier == runIdentifier, "panel identifier must be unique to this execution")
        require(panel.directoryURL?.standardizedFileURL == root.standardizedFileURL, "panel initial directory must be this execution's fixture root")

        let exact = validateWorkspaceSelection(expected: expected, selected: expected, fixtureRoot: root)
        require(exact.accepted, "exact generated chosen-workspace must be accepted")

        let mismatch = validateWorkspaceSelection(expected: expected, selected: different, fixtureRoot: root)
        require(!mismatch.accepted, "different sibling folder must be rejected")
        require(mismatch.diagnostic.contains(expected.standardizedFileURL.path), "mismatch diagnostic must include normalized expected fixture path")
        require(mismatch.diagnostic.contains(different.standardizedFileURL.path), "mismatch diagnostic must include normalized received fixture path")
        require(mismatch.diagnostic.contains("selected_basename=sibling-workspace"), "mismatch diagnostic must include selected basename")
        require(mismatch.diagnostic.contains("result=blocked"), "mismatch diagnostic must report blocked result")
        let rejection = workspaceSelectionDecision(mismatch)
        require(!rejection.shouldLaunchHelper, "selection mismatch decision must block helper launch")
        let expectedRejectionMessage = "BLOCKED: workspace_selection expected=\(expected.standardizedFileURL.path) received=\(different.standardizedFileURL.path) selected_basename=sibling-workspace result=blocked"
        require(rejection.userMessage == expectedRejectionMessage, "user-visible rejection must retain blocked status and both sanitized fixture paths")
        var helperInvocationCount = 0
        if rejection.shouldLaunchHelper { helperInvocationCount += 1 }
        require(helperInvocationCount == 0, "helper must not be invoked after a selection mismatch")

        let alias = root.appendingPathComponent("chosen-workspace-alias", isDirectory: true)
        try manager.createSymbolicLink(at: alias, withDestinationURL: expected)
        let aliased = validateWorkspaceSelection(expected: expected, selected: alias, fixtureRoot: root)
        require(aliased.accepted, "an existing symlink alias to the exact chosen-workspace target must be accepted after canonicalization")

        let outside = manager.temporaryDirectory.appendingPathComponent("unrelated-user-folder-\(UUID().uuidString)", isDirectory: true)
        try manager.createDirectory(at: outside, withIntermediateDirectories: true)
        defer { try? manager.removeItem(at: outside) }
        let externalMismatch = validateWorkspaceSelection(expected: expected, selected: outside, fixtureRoot: root)
        require(!externalMismatch.accepted, "folder outside this run fixture must be rejected")
        require(externalMismatch.diagnostic.contains("received=<outside temporary fixture>"), "unrelated path must not be included in diagnostic")
        let externalRejection = workspaceSelectionDecision(externalMismatch)
        require(externalRejection.userMessage.contains("expected=" + expected.standardizedFileURL.path), "external mismatch message must retain the sanitized expected path")
        require(externalRejection.userMessage.contains("received=<outside temporary fixture>"), "external mismatch message must retain the sanitized received path")
        require(!externalRejection.userMessage.contains(outside.path), "external mismatch message must not expose unrelated path")

        let wrongExpected = root.appendingPathComponent("not-chosen-workspace", isDirectory: true)
        try manager.createDirectory(at: wrongExpected, withIntermediateDirectories: true)
        let wrongFixture = validateWorkspaceSelection(expected: wrongExpected, selected: wrongExpected, fixtureRoot: root)
        require(!wrongFixture.accepted, "expected folder must remain this run's generated chosen-workspace")

        print("PASS: workspace selection validation")
    }
}
