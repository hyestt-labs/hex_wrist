// Golden vectors for `src/watch/e2ee.rs`, produced with CryptoKit exactly as
// Wristper's PairingKey.swift, EncryptedBridge.swift, and
// EncryptedWatchMacBridge.swift do, from fixed keys and nonces. Run:
//
//     swift scripts/watch-e2ee-vectors.swift > tests/fixtures/watch-e2ee/vectors.json
//
// The Rust tests must open every ciphertext here and, sealing the same
// plaintext with the same key and nonce, produce the same bytes. Change a
// constant here only together with the Swift app.
//
// The heads are encoded with `JSONEncoder`, as the watch encodes them, plus
// `.sortedKeys` so this script reproduces its output: without it the key
// order changes in every process, so no receiver can match the watch's
// head bytes, only its fields.
import CryptoKit
import Foundation

let keySalt = Data("Wristper E2EE v1".utf8)
let keyInfo = Data("Wristper pairing key".utf8)

func hex(_ data: Data) -> String { data.map { String(format: "%02x", $0) }.joined() }
func bytes(_ hex: String) -> Data {
    var data = Data()
    var index = hex.startIndex
    while index < hex.endIndex {
        let next = hex.index(index, offsetBy: 2)
        data.append(UInt8(hex[index..<next], radix: 16)!)
        index = next
    }
    return data
}

// Fixed scalars (any value below the curve order works; these are arbitrary).
let watchPrivate = try! P256.KeyAgreement.PrivateKey(
    rawRepresentation: bytes("0102030405060708091011121314151617181920212223242526272829303132"))
let macPrivate = try! P256.KeyAgreement.PrivateKey(
    rawRepresentation: bytes("a1a2a3a4a5a6a7a8a9b0b1b2b3b4b5b6b7b8b9c0c1c2c3c4c5c6c7c8c9d0d1d2"))

// PairingKey.sharedKey, from the Mac's side.
let secret = try! macPrivate.sharedSecretFromKeyAgreement(with: watchPrivate.publicKey)
let sharedKey = secret.hkdfDerivedSymmetricKey(using: SHA256.self, salt: keySalt, sharedInfo: keyInfo,
                                               outputByteCount: 32).withUnsafeBytes { Data($0) }
let symmetric = SymmetricKey(data: sharedKey)

func nonce(_ hexText: String) -> AES.GCM.Nonce { try! AES.GCM.Nonce(data: bytes(hexText)) }
func nonceData(_ nonce: AES.GCM.Nonce) -> Data { nonce.withUnsafeBytes { Data($0) } }

// PairingKey.sealResult: combined (nonce || ciphertext || tag) base64.
let requestID = "3f2a9c1e0b7d4e8fa6c5d4b3a2910f8e"
let token = "QmFzZTY0VXJsVG9rZW5PZjMyQnl0ZXNfLV8tXy0tXy0"
let resultNonce = nonce("0a0b0c0d0e0f101112131415")
let resultBox = try! AES.GCM.seal(Data(token.utf8), using: symmetric, nonce: resultNonce,
                                  authenticating: Data("Wristper pairing result v1\n\(requestID)".utf8))

// The watch's heads (WatchShared/WristperProtocol.swift), field for field.
struct AudioHead: Codable, Equatable {
    let version: Int
    let token: String
    let id: String
    let language: String
    let sentAt: Double
    let focus: String
    var route: String? = nil
    var session: String? = nil
    var project: String? = nil
    var requiresE2EE: Bool? = nil
    var background: Bool? = nil
    var approval: String? = nil

    enum CodingKeys: String, CodingKey {
        case version, token, id, language, focus, route, session, project, background, approval
        case sentAt = "sent_at"
        case requiresE2EE = "requires_e2ee"
    }
}

struct QueryHead: Codable, Equatable {
    let version: Int
    let token: String
    let sentAt: Double
    let path: String
    let query: [String: String]
    var requiresE2EE: Bool? = nil

    enum CodingKeys: String, CodingKey {
        case version, token, path, query
        case sentAt = "sent_at"
        case requiresE2EE = "requires_e2ee"
    }
}

/// The sorted bytes, after checking the watch's own (unsorted) encoding
/// carries the same object.
func encodeLikeTheWatch<Head: Encodable>(_ head: Head) -> Data {
    let sorted = JSONEncoder()
    sorted.outputFormatting = [.sortedKeys]
    let data = try! sorted.encode(head)
    let watch = try! JSONEncoder().encode(head)
    let object = { (bytes: Data) in try! JSONSerialization.jsonObject(with: bytes) as! NSDictionary }
    precondition(object(watch) == object(data), "sorting changed the head")
    return data
}

// EncryptedWatchMacBridge.seal: 4-byte big-endian head length, head JSON, audio.
let deviceID = "watch-device-7f3e"
let headData = encodeLikeTheWatch(AudioHead(
    version: 1, token: token, id: "6F9619FF-8B86-D011-B42D-00C04FC964FF", language: "zh",
    sentAt: 1762259696.5, focus: "requested", requiresE2EE: true, background: true))
let headJSON = String(decoding: headData, as: UTF8.self)
let audio = bytes("00000020667479704d3441200000000000006d7034326973366d0000000866726565")
var audioPlain = Data()
var headLength = UInt32(headData.count).bigEndian
withUnsafeBytes(of: &headLength) { audioPlain.append(contentsOf: $0) }
audioPlain.append(headData)
audioPlain.append(audio)
let audioNonce = nonce("202122232425262728292a2b")
let audioNonceText = nonceData(audioNonce).base64EncodedString()
let audioBox = try! AES.GCM.seal(
    audioPlain, using: symmetric, nonce: audioNonce,
    authenticating: Data("Wristper E2EE request v1\n\(deviceID)\n\(audioNonceText)".utf8))

// QueryHead sealed under the query AAD.
let queryData = encodeLikeTheWatch(QueryHead(
    version: 1, token: token, sentAt: 1762259700.25, path: "/status",
    query: ["id": "6F9619FF-8B86-D011-B42D-00C04FC964FF"]))
let queryJSON = String(decoding: queryData, as: UTF8.self)
let queryNonce = nonce("303132333435363738393a3b")
let queryNonceText = nonceData(queryNonce).base64EncodedString()
let queryBox = try! AES.GCM.seal(
    queryData, using: symmetric, nonce: queryNonce,
    authenticating: Data("Wristper E2EE query v1\n\(deviceID)\n\(queryNonceText)".utf8))

// EncryptedBridge.seal for the response, bound to the request nonce and status.
let responseStatus = 200
let responseJSON = #"{"e2ee_actions":true,"ok":true,"state":"unverified","verified":false}"#
let responseNonce = nonce("404142434445464748494a4b")
let responseBox = try! AES.GCM.seal(
    Data(responseJSON.utf8), using: symmetric, nonce: responseNonce,
    authenticating: Data("Wristper E2EE response v1\n\(deviceID)\n\(audioNonceText)\n\(responseStatus)".utf8))

let vectors: [String: Any] = [
    "watch_private_key_hex": hex(watchPrivate.rawRepresentation),
    "watch_public_key_x963_base64": watchPrivate.publicKey.x963Representation.base64EncodedString(),
    "mac_private_key_hex": hex(macPrivate.rawRepresentation),
    "mac_public_key_x963_base64": macPrivate.publicKey.x963Representation.base64EncodedString(),
    "shared_key_hex": hex(sharedKey),
    "shared_key_base64": sharedKey.base64EncodedString(),
    "pairing_result": [
        "request_id": requestID,
        "token": token,
        "nonce_hex": hex(nonceData(resultNonce)),
        "combined_base64": resultBox.combined!.base64EncodedString(),
    ],
    "audio": [
        "device_id": deviceID,
        "nonce_base64": audioNonceText,
        "head_json": headJSON,
        "audio_hex": hex(audio),
        "plaintext_base64": audioPlain.base64EncodedString(),
        "ciphertext_and_tag_base64": (audioBox.ciphertext + audioBox.tag).base64EncodedString(),
    ],
    "query": [
        "device_id": deviceID,
        "nonce_base64": queryNonceText,
        "plaintext": queryJSON,
        "ciphertext_and_tag_base64": (queryBox.ciphertext + queryBox.tag).base64EncodedString(),
    ],
    "response": [
        "device_id": deviceID,
        "request_nonce_base64": audioNonceText,
        "status": responseStatus,
        "plaintext": responseJSON,
        "nonce_base64": nonceData(responseNonce).base64EncodedString(),
        "ciphertext_base64": (responseBox.ciphertext + responseBox.tag).base64EncodedString(),
    ],
]
let output = try! JSONSerialization.data(withJSONObject: vectors, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
print(String(decoding: output, as: UTF8.self))
