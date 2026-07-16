import AppKit
import CoreServices
import Darwin
import Foundation
import IMsgCore
import PhoneNumberKit

let protocolVersion = 1
let maximumInputLineBytes = 1_048_576

struct BridgeRequest: Decodable, Equatable {
  let version: Int
  let id: String
  let command: String
  let recipient: String?
  let chatID: Int64?
  let chatGUID: String?
  let text: String?
  let routeID: String?
  let afterRowID: Int64?

  enum CodingKeys: String, CodingKey {
    case version, id, command, recipient, text
    case chatID = "chat_id"
    case chatGUID = "chat_guid"
    case routeID = "route_id"
    case afterRowID = "after_row_id"
  }
}

final class JSONOutput: @unchecked Sendable {
  private let lock = NSLock()

  func emit(_ value: [String: Any]) {
    guard JSONSerialization.isValidJSONObject(value),
      let data = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
    else {
      return
    }
    lock.lock()
    defer { lock.unlock() }
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data([0x0A]))
  }
}

enum BridgeFailure: Error {
  case invalidRequest(String)
  case protocolMismatch(Int)
  case notConfigured
  case fullDiskAccess
  case automation
  case messagesSignIn
  case sentMessageNotObserved
  case internalFailure

  var code: String {
    switch self {
    case .invalidRequest: return "invalid_request"
    case .protocolMismatch: return "protocol_mismatch"
    case .notConfigured: return "not_configured"
    case .fullDiskAccess: return "full_disk_access_required"
    case .automation: return "automation_required"
    case .messagesSignIn: return "messages_sign_in_required"
    case .sentMessageNotObserved: return "sent_message_not_observed"
    case .internalFailure: return "internal_error"
    }
  }

  var safeMessage: String {
    switch self {
    case .invalidRequest(let reason): return reason
    case .protocolMismatch(let version): return "Unsupported protocol version \(version)."
    case .notConfigured: return "The Messages bridge is not configured."
    case .fullDiskAccess: return "Clinch needs Full Disk Access to read the Messages database."
    case .automation: return "Clinch needs permission to automate Messages."
    case .messagesSignIn: return "Sign in to iMessage in Messages and try again."
    case .sentMessageNotObserved:
      return "Messages accepted the send, but Clinch could not identify the sent message."
    case .internalFailure: return "The Messages bridge encountered an internal error."
    }
  }

  var permission: String? {
    switch self {
    case .fullDiskAccess: return "full_disk_access"
    case .automation: return "automation"
    case .messagesSignIn: return "messages_sign_in"
    default: return nil
    }
  }
}

actor BridgeRuntime {
  private static let maximumRecentOutboundGUIDs = 4_096

  private let output: JSONOutput
  private var store: MessageStore?
  private var recipient = ""
  private var chatID: Int64?
  private var chatGUID = ""
  private var watchTask: Task<Void, Never>?
  private var sendCorrelationInProgress = false
  private var bufferedWatchMessages: [Message] = []
  private var recentOutboundGUIDs: Set<String> = []
  private var recentOutboundGUIDOrder: [String] = []

  init(output: JSONOutput) {
    self.output = output
  }

  func handle(_ request: BridgeRequest) async -> Bool {
    guard request.version == protocolVersion else {
      emitFailure(request.id, .protocolMismatch(request.version))
      return true
    }

    do {
      switch request.command {
      case "health":
        emitSuccess(request.id, result: await healthResult())
      case "request_automation":
        _ = await Self.requestAutomationAuthorization()
        emitSuccess(request.id, result: await healthResult())
      case "configure":
        try await configure(request)
        emitSuccess(request.id, result: configuredResult())
      case "send":
        let text = try requiredNonempty(request.text, name: "text")
        let sent = try await send(text: text)
        var result: [String: Any] = [
          "type": "sent",
          "guid": sent.message.guid,
          "row_id": sent.message.rowID,
          "chat_id": sent.message.chatID,
        ]
        if !sent.chatGUID.isEmpty { result["chat_guid"] = sent.chatGUID }
        if !sent.chatIdentifier.isEmpty { result["chat_identifier"] = sent.chatIdentifier }
        emitSuccess(request.id, result: result)
        finishSendCorrelation()
      case "start_watch":
        guard let requestedChatID = request.chatID, requestedChatID > 0 else {
          throw BridgeFailure.invalidRequest("start_watch requires a positive chat_id.")
        }
        try startWatch(chatID: requestedChatID, afterRowID: request.afterRowID ?? 0)
        emitSuccess(request.id, result: ["type": "watching"])
      case "stop_watch":
        stopWatch()
        emitSuccess(request.id, result: ["type": "stopped"])
      case "shutdown":
        stopWatch()
        emitSuccess(request.id, result: ["type": "shutdown"])
        return false
      default:
        throw BridgeFailure.invalidRequest("Unknown bridge command.")
      }
    } catch let failure as BridgeFailure {
      emitFailure(request.id, failure)
      finishSendCorrelation()
    } catch {
      emitFailure(request.id, classify(error))
      finishSendCorrelation()
    }
    return true
  }

  private func configure(_ request: BridgeRequest) async throws {
    recipient = try requiredNonempty(request.recipient, name: "recipient")
    chatID = request.chatID
    chatGUID = request.chatGUID?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""

    // Opening the database before sending ensures a missing Full Disk Access
    // grant fails before Apple Events can produce an uncorrelated side effect.
    let messageStore = try openStore()
    guard await Self.automationAuthorized(askUserIfNeeded: false) else {
      throw BridgeFailure.automation
    }
    guard await Self.imessageServiceAvailable() else { throw BridgeFailure.messagesSignIn }
    if chatID == nil,
      let chat = try messageStore.chatInfo(matchingTarget: recipient, preferredServices: ["iMessage"])
    {
      chatID = chat.id
      chatGUID = chat.guid
    }
  }

  private func configuredResult() -> [String: Any] {
    var result: [String: Any] = [
      "type": "configured",
      "recipient": recipient,
    ]
    if let chatID { result["chat_id"] = chatID }
    if !chatGUID.isEmpty { result["chat_guid"] = chatGUID }
    return result
  }

  private func healthStatus() async -> (
    messagesRunning: Bool, databaseReadable: Bool, automationAuthorized: Bool,
    imessageAvailable: Bool?
  ) {
    let messagesRunning = NSWorkspace.shared.runningApplications.contains {
      $0.bundleIdentifier == "com.apple.MobileSMS"
    }
    let databaseReadable: Bool
    do {
      _ = try openStore().maxRowID()
      databaseReadable = true
    } catch {
      databaseReadable = false
    }
    let automationAuthorized = await Self.automationAuthorized(askUserIfNeeded: false)
    let imessageAvailable = automationAuthorized ? await Self.imessageServiceAvailable() : nil
    return (messagesRunning, databaseReadable, automationAuthorized, imessageAvailable)
  }

  private func healthResult() async -> [String: Any] {
    let health = await healthStatus()
    var result: [String: Any] = [
      "type": "health",
      "messages_running": health.messagesRunning,
      "database_readable": health.databaseReadable,
      "automation_authorized": health.automationAuthorized,
    ]
    if let imessageAvailable = health.imessageAvailable {
      result["imessage_available"] = imessageAvailable
    }
    return result
  }

  @MainActor
  private static func automationAuthorized(askUserIfNeeded: Bool) -> Bool {
    let target = NSAppleEventDescriptor(bundleIdentifier: "com.apple.MobileSMS")
    guard let descriptor = target.aeDesc else { return false }
    return AEDeterminePermissionToAutomateTarget(
      descriptor, typeWildCard, typeWildCard, askUserIfNeeded
    ) == noErr
  }

  @MainActor
  private static func requestAutomationAuthorization() -> Bool {
    automationAuthorized(askUserIfNeeded: true)
  }

  // Apple documents NSAppleScript as main-thread-only. Both the service probe
  // and MessageSender's AppleScript implementation must stay on MainActor;
  // running them on BridgeRuntime's generic actor can hang until the parent
  // reaches its request timeout.
  @MainActor
  private static func imessageServiceAvailable() -> Bool {
    let script = NSAppleScript(
      source: "tell application \"Messages\" to count (every service whose service type is iMessage)"
    )
    var details: NSDictionary?
    let result = script?.executeAndReturnError(&details)
    guard details == nil else { return false }
    return (result?.int32Value ?? 0) > 0
  }

  @MainActor
  private static func sendViaMessages(_ options: MessageSendOptions) throws {
    try MessageSender().send(options)
  }

  private func openStore() throws -> MessageStore {
    if let store { return store }
    do {
      let opened = try MessageStore()
      _ = try opened.maxRowID()
      store = opened
      return opened
    } catch {
      throw classify(error)
    }
  }

  private struct SentResult {
    let message: Message
    let chatGUID: String
    let chatIdentifier: String
  }

  private func send(text: String) async throws -> SentResult {
    guard !recipient.isEmpty else { throw BridgeFailure.notConfigured }
    let messageStore = try openStore()
    let baseline = try messageStore.maxRowID()
    let startedAt = Date().addingTimeInterval(-2)
    sendCorrelationInProgress = true
    do {
      try await Self.sendViaMessages(
        MessageSendOptions(
          recipient: recipient,
          text: text,
          service: .imessage,
          chatGUID: chatGUID,
          allowSMSFallback: false
        ))
    } catch {
      throw classify(error)
    }

    for _ in 0..<80 {
      if let message = try messageStore.latestSentMessage(
        matchingText: text,
        chatID: chatID,
        since: startedAt
      ), message.rowID > baseline, !message.guid.isEmpty {
        recordOutboundGUID(message.guid)
        let info = try messageStore.chatInfo(chatID: message.chatID)
        chatID = message.chatID
        if let info, !info.guid.isEmpty { chatGUID = info.guid }
        return SentResult(
          message: message,
          chatGUID: info?.guid ?? chatGUID,
          chatIdentifier: info?.identifier ?? ""
        )
      }
      try await Task.sleep(for: .milliseconds(125))
    }

    // The watcher can win the database race even when the polling query does
    // not. Use its exact outgoing row as the final correlation source before
    // declaring delivery indeterminate.
    if let message = bufferedWatchMessages.first(where: {
      $0.rowID > baseline && $0.isFromMe && $0.text == text && !$0.guid.isEmpty
    }) {
      recordOutboundGUID(message.guid)
      let info = try messageStore.chatInfo(chatID: message.chatID)
      chatID = message.chatID
      if let info, !info.guid.isEmpty { chatGUID = info.guid }
      return SentResult(
        message: message,
        chatGUID: info?.guid ?? chatGUID,
        chatIdentifier: info?.identifier ?? ""
      )
    }
    throw BridgeFailure.sentMessageNotObserved
  }

  private func startWatch(chatID: Int64, afterRowID: Int64) throws {
    stopWatch()
    let watcher = MessageWatcher(store: try openStore())
    let stream = watcher.stream(
      chatID: chatID,
      sinceRowID: afterRowID,
      configuration: MessageWatcherConfiguration(
        debounceInterval: 0.25,
        fallbackPollInterval: 5,
        batchLimit: 100,
        includeReactions: true
      )
    )
    let output = output
    watchTask = Task { [weak self] in
      do {
        for try await message in stream {
          await self?.handleWatchedMessage(message)
        }
      } catch is CancellationError {
        return
      } catch {
        output.emit([
          "event": "watch_failed",
          "version": protocolVersion,
          "code": self?.classify(error).code ?? BridgeFailure.internalFailure.code,
        ])
      }
    }
  }

  private func handleWatchedMessage(_ message: Message) {
    guard message.service.caseInsensitiveCompare("iMessage") == .orderedSame else { return }
    if sendCorrelationInProgress {
      bufferedWatchMessages.append(message)
      return
    }
    emitWatchedMessage(message)
  }

  private func flushBufferedWatchMessages() {
    let buffered = bufferedWatchMessages
    bufferedWatchMessages.removeAll(keepingCapacity: true)
    for message in buffered {
      emitWatchedMessage(message)
    }
  }

  private func finishSendCorrelation() {
    guard sendCorrelationInProgress else { return }
    sendCorrelationInProgress = false
    flushBufferedWatchMessages()
  }

  private func emitWatchedMessage(_ message: Message) {
    if recentOutboundGUIDs.remove(message.guid) != nil {
      recentOutboundGUIDOrder.removeAll { $0 == message.guid }
      return
    }
    var normalized: [String: Any] = [
      "guid": message.guid,
      "row_id": message.rowID,
      "text": message.text,
      "service": message.service,
      "timestamp": ISO8601DateFormatter().string(from: message.date),
      "is_reaction": message.isReaction,
      "is_edited": false,
      "has_attachments": message.attachmentsCount > 0,
      "is_from_me": message.isFromMe,
    ]
    if let parent = message.replyToGUID ?? message.threadOriginatorGUID, !parent.isEmpty {
      normalized["parent_guid"] = parent
    }
    if let associated = message.threadOriginatorGUID, !associated.isEmpty {
      normalized["associated_guid"] = associated
    }
    output.emit([
      "event": "incoming",
      "version": protocolVersion,
      "message": normalized,
    ])
  }

  private func recordOutboundGUID(_ guid: String) {
    guard !guid.isEmpty, recentOutboundGUIDs.insert(guid).inserted else { return }
    recentOutboundGUIDOrder.append(guid)
    while recentOutboundGUIDOrder.count > Self.maximumRecentOutboundGUIDs {
      let expired = recentOutboundGUIDOrder.removeFirst()
      recentOutboundGUIDs.remove(expired)
    }
  }

  private func stopWatch() {
    watchTask?.cancel()
    watchTask = nil
  }

  private func requiredNonempty(_ value: String?, name: String) throws -> String {
    let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    guard !trimmed.isEmpty else {
      throw BridgeFailure.invalidRequest("\(name) must not be empty.")
    }
    return value ?? ""
  }

  nonisolated private func classify(_ error: Error) -> BridgeFailure {
    if let failure = error as? BridgeFailure { return failure }
    if case IMsgError.permissionDenied = error { return .fullDiskAccess }
    let description = error.localizedDescription.lowercased()
    if description.contains("not authorized") || description.contains("-1743") {
      return .automation
    }
    if description.contains("service") && description.contains("imessage") {
      return .messagesSignIn
    }
    return .internalFailure
  }

  private func emitSuccess(_ id: String, result: [String: Any]) {
    output.emit([
      "version": protocolVersion,
      "id": id,
      "ok": true,
      "result": result,
    ])
  }

  private func emitFailure(_ id: String, _ failure: BridgeFailure) {
    if let permission = failure.permission {
      output.emit([
        "event": "permission_required",
        "version": protocolVersion,
        "permission": permission,
      ])
    }
    output.emit([
      "version": protocolVersion,
      "id": id,
      "ok": false,
      "error": [
        "code": failure.code,
        "message": failure.safeMessage,
      ],
    ])
  }
}

@main
struct ClinchIMessageBridge {
  static func main() async {
    if CommandLine.arguments.dropFirst() == ["--protocol-version"] {
      print(protocolVersion)
      return
    }
    if CommandLine.arguments.dropFirst() == ["--self-test"] {
      let phoneNumbers = PhoneNumberUtility()
      guard
        let parsed = try? phoneNumbers.parse(
          "+14155551212",
          withRegion: "US",
          ignoreType: true
        ),
        phoneNumbers.format(parsed, toType: .e164) == "+14155551212"
      else {
        fputs("clinch-imessage-bridge could not load PhoneNumberKit resources\n", stderr)
        exit(EXIT_FAILURE)
      }
      print("clinch-imessage-bridge protocol \(protocolVersion)")
      return
    }

    let output = JSONOutput()
    let runtime = BridgeRuntime(output: output)
    let decoder = JSONDecoder()
    while let line = readLine(strippingNewline: true) {
      guard line.utf8.count <= maximumInputLineBytes else {
        output.emit([
          "version": protocolVersion,
          "id": "",
          "ok": false,
          "error": ["code": "line_too_large", "message": "Bridge request is too large."],
        ])
        continue
      }
      guard let data = line.data(using: .utf8) else { continue }
      do {
        let request = try decoder.decode(BridgeRequest.self, from: data)
        if !(await runtime.handle(request)) { break }
      } catch {
        output.emit([
          "version": protocolVersion,
          "id": "",
          "ok": false,
          "error": ["code": "invalid_json", "message": "Could not decode bridge request."],
        ])
      }
    }
  }
}
