import Foundation

struct ProbeResult: Codable {
    let role: String
    let resource: String
    let operation: String
    let succeeded: Bool
    let error: String?
}

func probe(role: String, resource: URL, operation: String) -> ProbeResult {
    let fm = FileManager.default
    let marker = resource.appendingPathComponent("marker.txt")
    do {
        switch operation {
        case "read":
            _ = try String(contentsOf: marker, encoding: .utf8)
        case "list":
            _ = try fm.contentsOfDirectory(atPath: resource.path)
        case "create":
            let created = resource.appendingPathComponent("created-\(UUID().uuidString).txt")
            try Data("created".utf8).write(to: created, options: .withoutOverwriting)
        case "modify":
            try Data("modified".utf8).write(to: marker)
        case "replace":
            let replacement = resource.appendingPathComponent("replacement-\(UUID().uuidString).tmp")
            try Data("replacement".utf8).write(to: replacement, options: .withoutOverwriting)
            _ = try fm.replaceItemAt(marker, withItemAt: replacement)
        default:
            throw NSError(domain: "Probe", code: 2, userInfo: [NSLocalizedDescriptionKey: "unknown operation"])
        }
        return ProbeResult(role: role, resource: resource.path, operation: operation, succeeded: true, error: nil)
    } catch {
        return ProbeResult(role: role, resource: resource.path, operation: operation, succeeded: false, error: String(describing: error))
    }
}

func requireDisposableFixture(_ path: String) throws {
    let standardized = URL(fileURLWithPath: path, isDirectory: true).standardizedFileURL
    let components = standardized.pathComponents
    guard let index = components.lastIndex(where: { $0.hasPrefix("huginn-app-sandbox-probe-") }) else {
        throw NSError(domain: "Probe", code: 3, userInfo: [NSLocalizedDescriptionKey: "resource is not in a generated baseline fixture; refusing access"])
    }
    let root = URL(fileURLWithPath: NSString.path(withComponents: Array(components.prefix(index + 1))), isDirectory: true)
    guard (try? String(contentsOf: root.appendingPathComponent(".probe-fixture"), encoding: .utf8)) == "disposable probe fixture\n" else {
        throw NSError(domain: "Probe", code: 4, userInfo: [NSLocalizedDescriptionKey: "baseline fixture marker is missing; refusing access"])
    }
}

func emit(_ result: ProbeResult) {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    let data = try! encoder.encode(result)
    print(String(decoding: data, as: UTF8.self))
}

let args = Array(CommandLine.arguments.dropFirst())
if args.count == 4, args[0] == "--probe" {
    do { try requireDisposableFixture(args[2]) } catch {
        fputs("BLOCKED/ERROR: \(error)\n", stderr)
        exit(1)
    }
    emit(probe(role: args[1], resource: URL(fileURLWithPath: args[2], isDirectory: true), operation: args[3]))
} else {
    fputs("usage: probe --probe ROLE RESOURCE OPERATION\n", stderr)
    exit(64)
}
