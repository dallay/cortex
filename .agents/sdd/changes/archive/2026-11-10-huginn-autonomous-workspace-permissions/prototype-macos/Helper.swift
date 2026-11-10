import Foundation

struct Result: Codable {
    let role: String
    let resource: String
    let operation: String
    let succeeded: Bool
    let error: String?
}

let operations = ["read", "list", "create", "modify", "replace"]

func probe(role: String, path: String, operation: String) -> Result {
    let url = URL(fileURLWithPath: path, isDirectory: true)
    let marker = url.appendingPathComponent("marker.txt")
    do {
        switch operation {
        case "read": _ = try Data(contentsOf: marker)
        case "list": _ = try FileManager.default.contentsOfDirectory(atPath: path)
        case "create": try Data("created".utf8).write(to: url.appendingPathComponent("created-\(UUID().uuidString)"), options: .withoutOverwriting)
        case "modify": try Data("modified".utf8).write(to: marker)
        case "replace":
            let replacement = url.appendingPathComponent("replacement-\(UUID().uuidString)")
            try Data("replacement".utf8).write(to: replacement, options: .withoutOverwriting)
            _ = try FileManager.default.replaceItemAt(marker, withItemAt: replacement)
        default: throw NSError(domain: "Probe", code: 1, userInfo: [NSLocalizedDescriptionKey: "unknown operation"])
        }
        return Result(role: role, resource: path, operation: operation, succeeded: true, error: nil)
    } catch {
        return Result(role: role, resource: path, operation: operation, succeeded: false, error: String(describing: error))
    }
}

func emit(_ result: Result) {
    let data = try! JSONEncoder().encode(result)
    print(String(decoding: data, as: UTF8.self))
    fflush(stdout)
}

func requireDisposableFixture(root: String, path: String) throws {
    let rootPath = URL(fileURLWithPath: root, isDirectory: true).standardizedFileURL.path
    let targetPath = URL(fileURLWithPath: path, isDirectory: true).standardizedFileURL.path
    guard targetPath == rootPath || targetPath.hasPrefix(rootPath.hasSuffix("/") ? rootPath : rootPath + "/") else {
        throw NSError(domain: "Prototype", code: 5, userInfo: [NSLocalizedDescriptionKey: "resource is outside host-provided disposable fixture root; refusing access"])
    }
}

func shellQuote(_ value: String) -> String { "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'" }

func run(_ process: Process, timeout: TimeInterval = 10) throws -> (Int32, String) {
    let pipe = Pipe()
    process.standardOutput = pipe
    process.standardError = pipe
    let lock = NSLock()
    var captured = Data()
    let cap = 16 * 1024
    pipe.fileHandleForReading.readabilityHandler = { handle in
        let data = handle.availableData
        guard !data.isEmpty else { return }
        lock.lock(); defer { lock.unlock() }
        if captured.count < cap { captured.append(data.prefix(cap - captured.count)) }
    }
    try process.run()
    let exited = DispatchSemaphore(value: 0)
    DispatchQueue.global().async { process.waitUntilExit(); exited.signal() }
    if exited.wait(timeout: .now() + timeout) == .timedOut { process.terminate(); throw NSError(domain: "Prototype", code: 7, userInfo: [NSLocalizedDescriptionKey: "child process timed out"]) }
    pipe.fileHandleForReading.readabilityHandler = nil
    let tail = pipe.fileHandleForReading.readDataToEndOfFile()
    lock.lock(); if captured.count < cap { captured.append(tail.prefix(cap - captured.count)) }; lock.unlock()
    return (process.terminationStatus, String(decoding: captured, as: UTF8.self))
}

func shellProbeScript(path: String) -> String {
    let p = shellQuote(path)
    let marker = shellQuote(URL(fileURLWithPath: path).appendingPathComponent("marker.txt").path)
    let resourceJSON = String(data: try! JSONEncoder().encode(path), encoding: .utf8)!
    return "operation=\"$1\"; case \"$operation\" in read) if IFS= read -r _ < \(marker); then probe_ok=true; else probe_ok=false; fi ;; list) if for entry in \(p)/*; do : \"$entry\"; done; then probe_ok=true; else probe_ok=false; fi ;; create) if : > \(shellQuote(URL(fileURLWithPath: path).appendingPathComponent("shell-created.txt").path)); then probe_ok=true; else probe_ok=false; fi ;; modify) if printf 'shell-modified\\n' > \(marker); then probe_ok=true; else probe_ok=false; fi ;; replace) : ;; *) exit 64 ;; esac; if [ \"$operation\" != replace ]; then printf '{\\\"role\\\":\\\"shell\\\",\\\"resource\\\":%s,\\\"operation\\\":\\\"%s\\\",\\\"succeeded\\\":%s,\\\"error\\\":%s}\\n' \(shellQuote(resourceJSON)) \"$operation\" \"$probe_ok\" \"$(if [ \"$probe_ok\" = true ]; then printf null; else printf '\\\"operation failed\\\"'; fi)\"; fi"
}

func runChain(root: String, path: String) throws {
    try requireDisposableFixture(root: root, path: path)
    emit(probe(role: "helper", path: path, operation: "read"))
    let helper = CommandLine.arguments[0]
    let script = shellProbeScript(path: path) + "; " + shellQuote(helper) + " --worker nested \(shellQuote(root)) \(shellQuote(path)) replace"
    let shell = Process(); shell.executableURL = URL(fileURLWithPath: "/bin/sh"); shell.arguments = ["-c", script, "prototype-shell", "modify"]
    let (status, output) = try run(shell)
    print(output, terminator: "")
    guard status == 0 else { throw NSError(domain: "Prototype", code: 8, userInfo: [NSLocalizedDescriptionKey: "shell chain exited \(status)"]) }
}

let args = Array(CommandLine.arguments.dropFirst())
if args.count == 5, args[0] == "--worker" {
    do { try requireDisposableFixture(root: args[2], path: args[3]) } catch {
        fputs("BLOCKED/ERROR: \(error)\n", stderr)
        exit(1)
    }
    emit(probe(role: args[1], path: args[3], operation: args[4]))
    exit(0)
}
if args.count == 3, args[0] == "--test-chain" {
    do { try runChain(root: args[1], path: args[2]); exit(0) }
    catch { fputs("BLOCKED/ERROR: \(error)\n", stderr); exit(1) }
}
guard args.count >= 4, args[0] == "--run", let bookmarkData = Data(base64Encoded: args[1]) else {
    fputs("invalid helper arguments; refusing to continue\n", stderr)
    exit(64)
}
var stale = false
do {
    let workspace = try URL(resolvingBookmarkData: bookmarkData, options: .withSecurityScope, relativeTo: nil, bookmarkDataIsStale: &stale)
    guard !stale, workspace.standardizedFileURL.path == args[2] else {
        throw NSError(domain: "Prototype", code: 2, userInfo: [NSLocalizedDescriptionKey: "stale bookmark or workspace mismatch"])
    }
    guard workspace.startAccessingSecurityScopedResource() else {
        throw NSError(domain: "Prototype", code: 3, userInfo: [NSLocalizedDescriptionKey: "startAccessingSecurityScopedResource returned false"])
    }
    defer { workspace.stopAccessingSecurityScopedResource() }
    print("bookmark_resolution=success helper_role=helper workspace=\(workspace.path)")
    let fixtureRoot = args[3]
    let targetArgs = Array(args.dropFirst(4))
    guard targetArgs.count.isMultiple(of: 2) else { throw NSError(domain: "Prototype", code: 4, userInfo: [NSLocalizedDescriptionKey: "malformed resource list"]) }
    let targets = stride(from: 0, to: targetArgs.count, by: 2).map { (targetArgs[$0], targetArgs[$0 + 1]) }
    for (name, path) in targets {
        try requireDisposableFixture(root: fixtureRoot, path: path)
        for operation in operations {
            emit(probe(role: "helper", path: path, operation: operation))
            let shellCommand = shellProbeScript(path: path) + "; " + shellQuote(CommandLine.arguments[0]) + " --worker nested \(shellQuote(fixtureRoot)) \(shellQuote(path)) \(shellQuote(operation))"
            let shell = Process()
            shell.executableURL = URL(fileURLWithPath: "/bin/sh")
            shell.arguments = ["-c", shellCommand]
            let (status, output) = try run(shell)
            print("resource_class=\(name) shell_exit=\(status)")
            print(output, terminator: "")
        }
    }
} catch {
    fputs("BLOCKED/ERROR: \(error)\n", stderr)
    exit(1)
}
