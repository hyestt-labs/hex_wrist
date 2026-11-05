import Foundation

/// A command-line Apple Watch: the shipping watch protocol code
/// (WatchMacPairing, EncryptedWatchMacBridge, WatchMacStatus, PairingKey)
/// driven against the production relay. Every line on stdout is one JSON
/// object; the last one is the result.
@main
struct WatchSim {
    static func main() async {
        let args = Array(CommandLine.arguments.dropFirst())
        do {
            switch args.first {
            case "pair": try await pair(timeout: Double(option(args, "--timeout") ?? "180") ?? 180)
            case "deliver": try await deliver(args)
            case "health": try await health()
            case "match": match(expected: option(args, "--expected") ?? "", actual: option(args, "--actual") ?? "")
            default:
                emit(["event": "usage", "usage": "watch-sim pair [--timeout s] | deliver --audio <m4a> [--language auto] [--enter] [--timeout s] | health | match --expected <text> --actual <text>"])
                exit(2)
            }
        } catch {
            emit(["event": "error", "error": "\(error)", "description": error.localizedDescription])
            exit(1)
        }
    }

    static func option(_ args: [String], _ name: String) -> String? {
        guard let index = args.firstIndex(of: name), index + 1 < args.count else { return nil }
        return args[index + 1]
    }

    static func emit(_ object: [String: Any]) {
        let data = (try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])) ?? Data("{}".utf8)
        FileHandle.standardOutput.write(data + Data("\n".utf8))
    }

    static func ms(since start: Date) -> Int { Int(Date().timeIntervalSince(start) * 1000) }

    static func pair(timeout: Double) async throws {
        let started = Date()
        let code = try await WatchMacPairing.startCode()
        emit(["event": "code", "code": code.code, "ttl": code.ttl, "device_id": WatchMacTargetStore.deviceID])
        let deadline = Date().addingTimeInterval(timeout)
        var claimed: (room: String, name: String)?
        while claimed == nil, Date() < deadline {
            if case .claimed(let room, let name) = try await WatchMacPairing.checkClaim(ticket: code.ticket) {
                claimed = (room, name)
            } else {
                try await Task.sleep(for: .milliseconds(1_500))
            }
        }
        guard let claimed else { throw PairError.timeout }
        emit(["event": "claimed", "room": claimed.room, "name": claimed.name, "ms": ms(since: started)])
        let (requestId, macDeviceId) = try await WatchMacPairing.requestPairing(room: claimed.room)
        let approval = try await WatchMacPairing.waitForApproval(room: claimed.room, requestId: requestId)
        guard WatchMacPairing.finish(room: claimed.room, name: claimed.name, macDeviceId: macDeviceId,
                                     requestId: requestId, approval: approval) else { throw PairingError.cannotSave }
        emit(["event": "paired", "ok": true, "room": claimed.room, "mac_name": claimed.name,
              "mac_device_id": macDeviceId, "encrypted": true, "ms": ms(since: started)])
    }

    /// How much of the expected sentence the read-back holds, ignoring case,
    /// punctuation, spacing, and Traditional versus Simplified characters.
    /// Words for Latin text; characters, in order, for CJK.
    static func match(expected: String, actual: String) {
        func fold(_ text: String) -> String {
            let simplified = text.applyingTransform(StringTransform("Hant-Hans"), reverse: false) ?? text
            return simplified.lowercased().unicodeScalars
                .map { CharacterSet.letters.union(.decimalDigits).contains($0) ? String($0) : " " }.joined()
        }
        let want = fold(expected), got = fold(actual)
        let cjk = want.unicodeScalars.contains { (0x4E00...0x9FFF).contains($0.value) }
        let score: Double
        if cjk {
            let a = Array(want.filter { !$0.isWhitespace }), b = Array(got.filter { !$0.isWhitespace })
            var row = [Int](repeating: 0, count: b.count + 1)
            for x in a {
                var previous = 0
                for (j, y) in b.enumerated() {
                    let saved = row[j + 1]
                    row[j + 1] = x == y ? previous + 1 : max(row[j + 1], row[j])
                    previous = saved
                }
            }
            score = a.isEmpty ? 0 : Double(row[b.count]) / Double(a.count)
        } else {
            let words = want.split(separator: " "), present = Set(got.split(separator: " "))
            score = words.isEmpty ? 0 : Double(words.filter(present.contains).count) / Double(words.count)
        }
        emit(["event": "match", "percent": Int((score * 100).rounded()), "mode": cjk ? "cjk-chars" : "words"])
    }

    static func health() async throws {
        guard let target = WatchMacTargetStore.selected() else { throw BridgeError.notConfigured }
        let health = await WatchMacStatus.health(room: target.room, token: target.token)
        emit(["event": "health", "ok": health.token == .valid, "token": "\(health.token)",
              "protocol": health.peer?.macProtocol as Any, "app_version": health.peer?.appVersion as Any])
    }

    /// One `/status` through `/e2ee/query`, as the watch asks it, with the
    /// decrypted body kept verbatim for the report.
    static func status(_ target: PairedMac, id: UUID) async -> (code: Int, body: [String: Any]?, state: String?) {
        guard case .encrypted(let query, let keyData)? = WatchMacStatus.statusRequest(target: target, id: id.uuidString),
              let reply = await EncryptedQuery.send(query, keyData: keyData) else { return (0, nil, nil) }
        let body = reply.data.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }
        let state = reply.decrypted ? reply.data.flatMap { try? JSONDecoder().decode(WatchMacStatusResult.self, from: $0) }?.state : nil
        return (reply.status, body, state)
    }

    /// WatchDelivery.swift `DeliveryPolicy.isTerminal`, minus the agent states.
    static let terminal: Set<String> = ["pasted", "unverified", "saved", "copied", "failed", "target-changed",
                                        "not-editable", "sensitive", "suspect", "segmented", "undone"]

    static func deliver(_ args: [String]) async throws {
        guard let path = option(args, "--audio") else { throw BridgeError.notConfigured }
        guard let target = WatchMacTargetStore.selected(), target.isEncrypted else { throw BridgeError.notConfigured }
        let audio = try Data(contentsOf: URL(fileURLWithPath: path))
        let language = option(args, "--language") ?? "auto"
        let timeout = Double(option(args, "--timeout") ?? "120") ?? 120
        let id = UUID()
        var report: [String: Any] = ["event": "delivered", "id": id.uuidString, "audio_bytes": audio.count]

        let started = Date()
        let refusal = await WatchMacStatus.captureFocus(target: target, id: id)
        report["focus_ms"] = ms(since: started)
        report["focus_refusal"] = refusal.map { "\($0)" } as Any

        let sendStart = Date()
        let result = try await WatchMacBridge.send(audio: audio, id: id, room: target.room, token: target.token,
                                                   language: language, e2eeKey: target.e2eeKey)
        report["send_ms"] = ms(since: sendStart)
        report["send"] = ["ok": result.ok, "state": result.state as Any, "verification": result.verification as Any,
                          "text": result.text as Any, "reason": result.reason as Any,
                          "transcribe_ms": result.transcribeMs as Any, "paste_ms": result.pasteMs as Any]

        var last: (code: Int, body: [String: Any]?, state: String?) = (0, nil, result.state)
        let deadline = Date().addingTimeInterval(timeout)
        var polls = 0
        repeat {
            try await Task.sleep(for: .milliseconds(1_500))
            last = await status(target, id: id)
            polls += 1
        } while !(last.state.map(terminal.contains) ?? false) && Date() < deadline
        report["status_code"] = last.code
        report["status"] = last.body as Any
        report["status_polls"] = polls
        report["state"] = last.state as Any
        report["terminal"] = last.state.map(terminal.contains) ?? false

        if args.contains("--enter") {
            let enterStart = Date()
            let entered = await WatchMacStatus.pressEnter(target: target, id: id)
            report["enter"] = entered == .entered ? "entered" : "\(entered)"
            report["enter_ms"] = ms(since: enterStart)
        }
        report["total_ms"] = ms(since: started)
        report["ok"] = report["terminal"] as? Bool ?? false
        emit(report)
    }
}
