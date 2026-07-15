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
