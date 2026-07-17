import LinguaMeshCore
import XCTest

final class LinguaMeshCoreTests: XCTestCase {
    func testCompatibilityAndLifecycle() throws {
        let compatibility = LinguaMeshEngine.queryCompatibility()
        XCTAssertEqual(compatibility.abiMajor, LinguaMeshEngine.abiVersionMajor)
        XCTAssertEqual(compatibility.protocolVersion, LinguaMeshEngine.protocolVersion)
        XCTAssertEqual(CoreResult(rawValue: 9), .resourceExhausted)
        let engine = try LinguaMeshEngine()
        requireSendable(engine)
        XCTAssertTrue(try engine.pollEvent(timeoutMilliseconds: 0).isEmpty)
        try engine.shutdown()
        try engine.close()
    }

    func testIncompatibleABIIsRejectedBeforeCreation() {
        XCTAssertThrowsError(
            try LinguaMeshEngine(expectedABI: LinguaMeshEngine.abiVersionMajor + 1)
        ) { error in
            XCTAssertTrue(error is CoreCompatibilityError)
        }
    }

    private func requireSendable<T: Sendable>(_ value: T) {
        _ = value
    }
}
