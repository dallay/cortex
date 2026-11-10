import AppKit
import Foundation

func collectProcessOutput(_ process: Process, timeout: TimeInterval = 15) throws -> String {
    let pipe = Pipe()
    process.standardOutput = pipe
    process.standardError = pipe
    let lock = NSLock()
    var output = Data()
    let limit = 64 * 1024
    pipe.fileHandleForReading.readabilityHandler = { handle in
        let data = handle.availableData
        guard !data.isEmpty else { return }
        lock.lock(); defer { lock.unlock() }
        if output.count < limit { output.append(data.prefix(limit - output.count)) }
    }
    try process.run()
    let finished = DispatchSemaphore(value: 0)
    DispatchQueue.global().async { process.waitUntilExit(); finished.signal() }
    guard finished.wait(timeout: .now() + timeout) == .success else {
        process.terminate()
        throw NSError(domain: "Prototype", code: 10, userInfo: [NSLocalizedDescriptionKey: "helper process timed out"])
    }
    pipe.fileHandleForReading.readabilityHandler = nil
    let tail = pipe.fileHandleForReading.readDataToEndOfFile()
    lock.lock(); if output.count < limit { output.append(tail.prefix(limit - output.count)) }; lock.unlock()
    guard process.terminationStatus == 0 else {
        throw NSError(domain: "Prototype", code: 11, userInfo: [NSLocalizedDescriptionKey: "helper exited \(process.terminationStatus): \(String(decoding: output, as: UTF8.self))"])
    }
    return String(decoding: output, as: UTF8.self)
}

struct HostProbeResult: Codable {
    let role: String
    let resource: String
    let operation: String
    let succeeded: Bool
    let error: String?

    init(resource: String, operation: String, succeeded: Bool, error: String?) {
        self.role = "host"
        self.resource = resource
        self.operation = operation
        self.succeeded = succeeded
        self.error = error
    }
}

func hostProbe(_ path: String, operation: String) -> HostProbeResult {
    let directory = URL(fileURLWithPath: path, isDirectory: true)
    let marker = directory.appendingPathComponent("marker.txt")
    do {
        switch operation {
        case "read": _ = try Data(contentsOf: marker)
        case "list": _ = try FileManager.default.contentsOfDirectory(atPath: path)
        case "create": try Data("created".utf8).write(to: directory.appendingPathComponent("host-created-\(UUID().uuidString)"), options: .withoutOverwriting)
        case "modify": try Data("modified".utf8).write(to: marker)
        case "replace":
            let replacement = directory.appendingPathComponent("host-replacement-\(UUID().uuidString)")
            try Data("replacement".utf8).write(to: replacement, options: .withoutOverwriting)
            _ = try FileManager.default.replaceItemAt(marker, withItemAt: replacement)
        default: throw NSError(domain: "Probe", code: 1, userInfo: [NSLocalizedDescriptionKey: "unknown operation"])
        }
        return HostProbeResult(resource: path, operation: operation, succeeded: true, error: nil)
    } catch {
        return HostProbeResult(resource: path, operation: operation, succeeded: false, error: String(describing: error))
    }
}

final class Host: NSObject, NSApplicationDelegate {
    private var window: NSWindow!
    private var output: NSTextView!
    private var fixtureRoot: URL!

    func applicationDidFinishLaunching(_ notification: Notification) {
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 500), styleMask: [.titled, .closable, .miniaturizable], backing: .buffered, defer: false)
        window.title = "App Sandbox Feasibility Prototype"
        let view = NSView(frame: window.contentView!.bounds)
        let button = NSButton(title: "Select disposable workspace and run probes…", target: self, action: #selector(selectWorkspace))
        button.frame = NSRect(x: 20, y: 450, width: 420, height: 30)
        output = NSTextView(frame: NSRect(x: 20, y: 20, width: 720, height: 410))
        output.isEditable = false
        view.addSubview(button)
        view.addSubview(output)
        window.contentView = view
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        if ProcessInfo.processInfo.arguments.contains("--terminal-entry") {
            DispatchQueue.main.async { self.selectWorkspace() }
        }
    }

    @objc private func selectWorkspace() {
        output.string = ""
        do {
            fixtureRoot = FileManager.default.temporaryDirectory.appendingPathComponent("huginn-app-sandbox-\(UUID().uuidString)", isDirectory: true)
            try FileManager.default.createDirectory(at: fixtureRoot, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: fixtureRoot) }
            try Data("disposable fixture root\n".utf8).write(to: fixtureRoot.appendingPathComponent(".prototype-fixture"))
            let chosen = fixtureRoot.appendingPathComponent("chosen-workspace", isDirectory: true)
            let unchosen = fixtureRoot.appendingPathComponent("unchosen-workspace", isDirectory: true)
            let grants = fixtureRoot.appendingPathComponent("fake-home/Library/Application Support/Huginn/grants", isDirectory: true)
            for url in [chosen, unchosen, grants] {
                try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
                try Data("disposable sentinel\n".utf8).write(to: url.appendingPathComponent("marker.txt"))
            }
            var targets: [(String, URL)] = [("chosen", chosen), ("unchosen", unchosen), ("grant-store", grants)]
            var ancestor = grants.deletingLastPathComponent()
            while ancestor.path.hasPrefix(fixtureRoot.path), ancestor != fixtureRoot {
                targets.append(("grant-ancestor", ancestor))
                ancestor.deleteLastPathComponent()
            }
            targets.append(("grant-ancestor", fixtureRoot))
            for (_, target) in targets {
                let marker = target.appendingPathComponent("marker.txt")
                if !FileManager.default.fileExists(atPath: marker.path) {
                    try Data("disposable sentinel\n".utf8).write(to: marker)
                }
            }

            let panel = NSOpenPanel()
            panel.title = "Select the generated chosen-workspace folder"
            panel.message = "Select only the disposable chosen-workspace folder created for this run. Do not select another folder."
            panel.canChooseDirectories = true
            panel.canChooseFiles = false
            panel.allowsMultipleSelection = false
            let runIdentifier = NSUserInterfaceItemIdentifier("huginn-workspace-selection-\(UUID().uuidString)")
            guard configureWorkspacePanel(panel, identifier: runIdentifier, fixtureRoot: fixtureRoot) else {
                throw NSError(domain: "Prototype", code: 2, userInfo: [NSLocalizedDescriptionKey: "could not establish the disposable fixture root as the panel's initial directory; no helper launched"])
            }
            guard panel.runModal() == .OK, let selected = panel.url else {
                throw NSError(domain: "Prototype", code: 1, userInfo: [NSLocalizedDescriptionKey: "workspace selection cancelled; no helper launched"])
            }
            let validation = validateWorkspaceSelection(expected: chosen, selected: selected, fixtureRoot: fixtureRoot)
            let decision = workspaceSelectionDecision(validation)
            output.string += decision.userMessage + "\n"
            guard decision.shouldLaunchHelper else {
                return
            }
            for (name, target) in targets {
                for operation in ["read", "list", "create", "modify", "replace"] {
                    let data = try JSONEncoder().encode(hostProbe(target.path, operation: operation))
                    output.string += String(decoding: data, as: UTF8.self) + " resource_class=\(name)\n"
                }
            }
            let bookmark = try selected.bookmarkData(options: .withSecurityScope, includingResourceValuesForKeys: nil, relativeTo: nil)
            let helper = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/ProbeHelper")
            let process = Process()
            process.executableURL = helper
            process.arguments = ["--run", bookmark.base64EncodedString(), selected.path, fixtureRoot.path] + targets.flatMap { [$0.0, $0.1.path] }
            output.string += try collectProcessOutput(process) + "\nhelper_exit=\(process.terminationStatus)\nfixture_root=\(fixtureRoot.path)\n"
        } catch {
            output.string += "BLOCKED/ERROR: \(error)\n"
        }
    }
}

let app = NSApplication.shared
let delegate = Host()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
