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
            case "action": try await action(args)
            case "selftest-gate": selftestGate(args)
            case "match": match(expected: option(args, "--expected") ?? "", actual: option(args, "--actual") ?? "")
            default:
                emit(["event": "usage", "usage": "watch-sim pair [--timeout s] | deliver --audio <m4a> [--language auto] [--after-focus <sh>] [--before-action <sh>] [--enter] [--enter-twice] [--undo] [--timeout s] | action --path /enter|/undo --id <uuid> | selftest-gate --hook <sh> | health | match --expected <text> --actual <text>"])
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

    /// `WATCH_SIM_DRY_RUN=1`: nothing goes over the network. Every step is recorded in `trace` instead,
    /// in the order it would happen, so the script's self-test can check the real order of the real code:
    /// each gate comes before the send it guards.
    static let dryRun = ProcessInfo.processInfo.environment["WATCH_SIM_DRY_RUN"] == "1"
    nonisolated(unsafe) static var traceLog: [String] = []
    static let traceLock = NSLock()
    static func trace(_ step: String) {
        traceLock.lock(); traceLog.append(step); traceLock.unlock()
    }

    /// Runs a gate command (`/bin/sh -c`) and reports whether it passed. The script uses it to check that
    /// the app in front is the test target at the moment audio or a key would go out; a failed gate means
    /// nothing is sent.
    static func gate(_ command: String?, label: String = "gate") -> Bool {
        trace("gate:\(label)")
        guard let command else { return false }   // no gate given: fail closed, send nothing
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/bin/sh")
        task.arguments = ["-c", command]
        do { try task.run() } catch { return false }
        task.waitUntilExit()
        return task.terminationStatus == 0
    }

    /// One /enter or /undo: the gate runs first, on its own, and only a pass sends it. Every send of
    /// an action goes through here, so none can skip its check.
    static func gatedAction(_ target: PairedMac?, _ path: String, id: UUID, hook: String?) async -> [String: Any] {
        guard gate(hook, label: path) else { return ["aborted": "before-action gate failed", "sent": false] }
        trace("send:\(path)")
        if dryRun || target == nil { return ["status": 200, "body": ["ok": true], "dry_run": true] }
        return await rawAction(target!, path, id: id)
    }

    /// `selftest-gate --hook <sh>`: the same gate with no network, for the script's offline self-test.
    /// Prints whether audio would be sent, and exits 0 only when the gate passed.
    static func selftestGate(_ args: [String]) {
        let passed = gate(option(args, "--hook"))
        emit(["event": "selftest-gate", "e2ee_sent": passed, "gate_passed": passed, "trace": traceLog])
        exit(passed ? 0 : 1)
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

    /// How much of the expected sentence the read-back holds, in order: the longest
    /// common subsequence of its words (Latin text) or characters (CJK) over the
    /// expected length, ignoring case, punctuation, spacing, and Traditional versus
    /// Simplified characters. Words that appear out of order do not count. An empty
    /// read-back, or one with fewer than half the expected units, scores 0 however
    /// well its few units match.
    static func match(expected: String, actual: String) {
        func fold(_ text: String) -> String {
            let simplified = text.applyingTransform(StringTransform("Hant-Hans"), reverse: false) ?? text
            return simplified.lowercased().unicodeScalars
                .map { CharacterSet.letters.union(.decimalDigits).contains($0) ? String($0) : " " }.joined()
        }
        func lcs<T: Equatable>(_ a: [T], _ b: [T]) -> Int {
            var row = [Int](repeating: 0, count: b.count + 1)
            for x in a {
                var previous = 0
                for (j, y) in b.enumerated() {
                    let saved = row[j + 1]
                    row[j + 1] = x == y ? previous + 1 : max(row[j + 1], row[j])
                    previous = saved
                }
            }
            return row[b.count]
        }
        let want = fold(expected), got = fold(actual)
        let cjk = want.unicodeScalars.contains { (0x4E00...0x9FFF).contains($0.value) }
        let wantUnits: [String], gotUnits: [String]
        if cjk {
            wantUnits = want.filter { !$0.isWhitespace }.map(String.init)
            gotUnits = got.filter { !$0.isWhitespace }.map(String.init)
        } else {
            wantUnits = want.split(separator: " ").map(String.init)
            gotUnits = got.split(separator: " ").map(String.init)
        }
        let tooShort = gotUnits.isEmpty || gotUnits.count * 2 < wantUnits.count
        let common = tooShort ? 0 : lcs(wantUnits, gotUnits)
        let score = wantUnits.isEmpty ? 0 : Double(common) / Double(wantUnits.count)
        // Precision: how much of the read-back is the sentence. Two copies of it score 50.
        let precision = gotUnits.isEmpty ? 0 : Double(common) / Double(gotUnits.count)
        emit(["event": "match", "percent": Int((score * 100).rounded()),
              "precision": Int((precision * 100).rounded()),
              "mode": cjk ? "ordered-chars" : "ordered-words", "common": common,
              "expected_units": wantUnits.count, "actual_units": gotUnits.count, "too_short": tooShort])
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

    /// One watch action (`/enter`, `/undo`) through the shipping watch code's own
    /// encrypted `sendAction`, with the Mac's reply kept verbatim.
    static func rawAction(_ target: PairedMac, _ path: String, id: UUID) async -> [String: Any] {
        switch await WatchMacStatus.sendAction(target: target, path: path, id: id, timeout: 20) {
        case .reply(let status, let body):
            let json = (try? JSONSerialization.jsonObject(with: body)) as? [String: Any]
            return ["status": status, "body": json ?? String(decoding: body, as: UTF8.self)]
        case .noReply: return ["status": 0, "error": "no-reply"]
        case .unauthenticated: return ["status": 401, "error": "unauthenticated"]
        case .relayFailure(let failure): return ["status": 0, "error": "\(failure)"]
        }
    }

    /// `action --path /enter|/undo --id <uuid> --before-action <sh>`: one action for an earlier recording.
    /// It needs the gate like every other send: without `--before-action`, or if it fails, nothing is sent.
    static func action(_ args: [String]) async throws {
        let target = WatchMacTargetStore.selected()
        guard dryRun || target?.isEncrypted == true,
              let path = option(args, "--path"), path == "/enter" || path == "/undo",
              let id = option(args, "--id").flatMap(UUID.init) else {
            throw BridgeError.notConfigured
        }
        var report = await gatedAction(target, path, id: id, hook: option(args, "--before-action"))
        report["event"] = "action"
        report["trace"] = traceLog
        emit(report)
        if report["aborted"] != nil { exit(1) }
    }

    /// WatchDelivery.swift `DeliveryPolicy.isTerminal`, minus the agent states.
    static let terminal: Set<String> = ["pasted", "unverified", "saved", "copied", "failed", "target-changed",
                                        "not-editable", "sensitive", "suspect", "segmented", "undone"]

    static func deliver(_ args: [String]) async throws {
        guard let path = option(args, "--audio") else { throw BridgeError.notConfigured }
        let target = WatchMacTargetStore.selected()
        guard dryRun || target?.isEncrypted == true else { throw BridgeError.notConfigured }
        let audio = dryRun ? Data() : try Data(contentsOf: URL(fileURLWithPath: path))
        let language = option(args, "--language") ?? "auto"
        let timeout = Double(option(args, "--timeout") ?? "120") ?? 120
        let id = UUID()
        var report: [String: Any] = ["event": "delivered", "id": id.uuidString, "audio_bytes": audio.count]

        let started = Date()
        trace("send:/focus")
        let refusal = dryRun ? nil : await WatchMacStatus.captureFocus(target: target!, id: id)
        report["focus_ms"] = ms(since: started)
        report["focus_refusal"] = refusal.map { "\($0)" } as Any
        // The person's app may change between starting the recording and its audio arriving. This gate runs
        // after /focus and before /e2ee. If it fails, nothing is sent: no audio, so no dictation.
        do {
            let passed = gate(option(args, "--after-focus"), label: "after-focus")
            report["after_focus_passed"] = passed
            if !passed {
                report["aborted"] = "after-focus gate failed"
                report["e2ee_sent"] = false
                report["ok"] = false
                report["trace"] = traceLog
                emit(report)
                exit(1)
            }
        }
        report["e2ee_sent"] = true

        let sendStart = Date()
        trace("send:/e2ee")
        var last: (code: Int, body: [String: Any]?, state: String?) = (0, nil, nil)
        if dryRun {
            last = (200, nil, "pasted")
            report["send"] = ["ok": true, "state": "pasted", "dry_run": true]
            report["send_ms"] = 0
        } else {
            let result = try await WatchMacBridge.send(audio: audio, id: id, room: target!.room, token: target!.token,
                                                       language: language, e2eeKey: target!.e2eeKey)
            report["send_ms"] = ms(since: sendStart)
            report["send"] = ["ok": result.ok, "state": result.state as Any, "verification": result.verification as Any,
                              "text": result.text as Any, "reason": result.reason as Any,
                              "transcribe_ms": result.transcribeMs as Any, "paste_ms": result.pasteMs as Any]
            last = (0, nil, result.state)
            let deadline = Date().addingTimeInterval(timeout)
            var polls = 0
            repeat {
                try await Task.sleep(for: .milliseconds(1_500))
                last = await status(target!, id: id)
                polls += 1
            } while !(last.state.map(terminal.contains) ?? false) && Date() < deadline
            report["status_polls"] = polls
        }
        report["status_code"] = last.code
        report["status"] = last.body as Any
        report["state"] = last.state as Any
        report["terminal"] = last.state.map(terminal.contains) ?? false

        // /enter and /undo make the Mac press Return and Command-Z. Each send is its own gated action: the
        // gate runs before it and only a pass sends it. Two Enters at the same moment are two actions, each
        // checked on its own.
        let beforeAction = option(args, "--before-action")
        if args.contains("--enter") {
            let enterStart = Date()
            report["enter"] = await gatedAction(target, "/enter", id: id, hook: beforeAction)
            report["enter_ms"] = ms(since: enterStart)
        }
        if args.contains("--enter-twice") {
            async let first = gatedAction(target, "/enter", id: id, hook: beforeAction)
            async let second = gatedAction(target, "/enter", id: id, hook: beforeAction)
            report["enters"] = await [first, second]
        }
        if args.contains("--undo") {
            // Undo needs the read-back to have confirmed the paste (up to 1 s after it).
            if !dryRun { try await Task.sleep(for: .seconds(2)) }
            report["undo"] = await gatedAction(target, "/undo", id: id, hook: beforeAction)
        }
        report["trace"] = traceLog
        report["total_ms"] = ms(since: started)
        report["ok"] = report["terminal"] as? Bool ?? false
        emit(report)
    }
}
