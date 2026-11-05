import CryptoKit
import Foundation

/// Stands in for the watch's Keychain-backed `WatchMacTargetStore`
/// (WatchProbe/WatchMacTargetStore.swift) with the same API, kept in one JSON
/// file under WATCH_SIM_STATE so the simulator never writes to the login
/// Keychain. Every other type the simulator uses is the shipping watch code.
enum WatchMacTargetStore {
    private struct State: Codable {
        var deviceID: String
        var pairings: [PairedMac] = []
        var pending: [String: Data] = [:]
        var encryptedActions: [String] = []
    }

    static let pendingE2EEPrefix = "pending-e2ee-"

    private static var file: URL {
        let dir = ProcessInfo.processInfo.environment["WATCH_SIM_STATE"]
            ?? FileManager.default.temporaryDirectory.appendingPathComponent("watch-sim").path
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        return URL(fileURLWithPath: dir).appendingPathComponent("watch-state.json")
    }

    private static func load() -> State {
        if let data = try? Data(contentsOf: file), let state = try? JSONDecoder().decode(State.self, from: data) {
            return state
        }
        let state = State(deviceID: UUID().uuidString)
        store(state)
        return state
    }

    private static func store(_ state: State) {
        guard let data = try? JSONEncoder().encode(state) else { return }
        try? data.write(to: file, options: .atomic)
        try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
    }

    static var deviceID: String { load().deviceID }

    static func pairings() -> [PairedMac] { load().pairings }

    static func selected() -> PairedMac? { load().pairings.last }

    @discardableResult
    static func save(_ pairing: PairedMac) -> Bool {
        var state = load()
        state.pairings.removeAll { $0.id == pairing.id }
        state.pairings.append(pairing)
        store(state)
        return true
    }

    private static func actionsAccount(_ key: Data) -> String {
        SHA256.hash(data: key).prefix(16).map { String(format: "%02x", $0) }.joined()
    }

    static func knowsEncryptedActions(key: Data) -> Bool {
        load().encryptedActions.contains(actionsAccount(key))
    }

    static func rememberEncryptedActions(key: Data) {
        var state = load()
        guard !state.encryptedActions.contains(actionsAccount(key)) else { return }
        state.encryptedActions.append(actionsAccount(key))
        store(state)
    }

    static func savePendingE2EEPrivate(_ data: Data, requestID: String) -> Bool {
        var state = load()
        state.pending[requestID] = data
        store(state)
        return true
    }

    static func pendingE2EEPrivate(requestID: String) -> Data? { load().pending[requestID] }

    static func pendingE2EEItems() -> [(account: String, created: Date)] {
        load().pending.keys.map { ("\(pendingE2EEPrefix)\($0)", Date()) }
    }

    static func clearPendingE2EEPrivate(requestID: String) {
        var state = load()
        state.pending[requestID] = nil
        store(state)
    }
}

/// WatchMacStatus.swift names these agent types (AgentRun.swift) in its
/// agent functions; the simulator never calls them, so empty shapes suffice.
enum AgentRunList {
    struct Item: Decodable {}
    struct Reply: Decodable { let runs: [Item] }
}

enum AgentRunLookup {
    case found(WatchMacStatusResult)
    case failed
    init(failure: Error) { self = .failed }
}
