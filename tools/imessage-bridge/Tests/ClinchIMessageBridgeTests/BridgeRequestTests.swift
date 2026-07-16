import Foundation
import Testing

@testable import ClinchIMessageBridge

@Test func decodesVersionedSnakeCaseRequest() throws {
  let data = Data(
    #"{"version":1,"id":"42","command":"start_watch","chat_id":9,"after_row_id":17}"#.utf8
  )
  let request = try JSONDecoder().decode(BridgeRequest.self, from: data)
  #expect(request.version == 1)
  #expect(request.id == "42")
  #expect(request.command == "start_watch")
  #expect(request.chatID == 9)
  #expect(request.afterRowID == 17)
}

@Test func bridgeRequestRejectsMissingProtocolVersion() {
  let data = Data(#"{"id":"42","command":"health"}"#.utf8)
  #expect(throws: DecodingError.self) {
    try JSONDecoder().decode(BridgeRequest.self, from: data)
  }
}

@Test func decodesExplicitAutomationRequest() throws {
  let data = Data(#"{"version":1,"id":"permission","command":"request_automation"}"#.utf8)
  let request = try JSONDecoder().decode(BridgeRequest.self, from: data)
  #expect(request.command == "request_automation")
}

@Test func normalizesMessagesRecipientToE164() throws {
  #expect(try normalizedMessagesRecipient("4155551212") == "+14155551212")
  #expect(try normalizedMessagesRecipient("+44 20 7946 0958") == "+442079460958")
}

@Test func messagesAppleScriptPassesRecipientAndTextAsArguments() {
  let recipient = "+14155551212"
  let text = "A quoted message: \"hello\""
  let arguments = MessagesAppleScript.arguments(recipient: recipient, text: text)

  #expect(arguments == ["-l", "AppleScript", "-", recipient, text])
  #expect(!MessagesAppleScript.source.contains(recipient))
  #expect(!MessagesAppleScript.source.contains(text))
}

@Test func mirroredSelfChatRowsAreCoalescedNarrowly() {
  let sent = MirroredMessageFingerprint(
    rowID: 10,
    chatID: 7,
    sender: "+14155551212",
    text: "continue",
    date: Date(timeIntervalSince1970: 100),
    isFromMe: true,
    service: "iMessage",
    attachmentsCount: 0
  )
  let mirrored = MirroredMessageFingerprint(
    rowID: 11,
    chatID: 7,
    sender: "+14155551212",
    text: "continue",
    date: Date(timeIntervalSince1970: 100.1),
    isFromMe: false,
    service: "iMessage",
    attachmentsCount: 0
  )
  #expect(sent.mirrors(mirrored))

  let sameDirection = MirroredMessageFingerprint(
    rowID: 12,
    chatID: 7,
    sender: "+14155551212",
    text: "continue",
    date: Date(timeIntervalSince1970: 100.2),
    isFromMe: true,
    service: "iMessage",
    attachmentsCount: 0
  )
  #expect(!sent.mirrors(sameDirection))

  let interleaved = MirroredMessageFingerprint(
    rowID: 12,
    chatID: 7,
    sender: "+14155551212",
    text: "continue",
    date: Date(timeIntervalSince1970: 100.2),
    isFromMe: false,
    service: "iMessage",
    attachmentsCount: 0
  )
  #expect(!sent.mirrors(interleaved))

  let laterReply = MirroredMessageFingerprint(
    rowID: 13,
    chatID: 7,
    sender: "+14155551212",
    text: "continue",
    date: Date(timeIntervalSince1970: 103),
    isFromMe: false,
    service: "iMessage",
    attachmentsCount: 0
  )
  #expect(!sent.mirrors(laterReply))
}
