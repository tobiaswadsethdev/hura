// Certificate pinning. An `hurad` signs its own certificate, so there is no
// authority to check it against; a connection is accepted if and only if the
// leaf certificate's SHA-256 is the one in the pairing string (hura's
// `docs/server.md`). The system trust store and host names play no part.

import CryptoKit
import Foundation
import Security

public enum Pinning {
    /// Lowercase hex SHA-256 of the whole DER certificate, the form `hurad
    /// pair` prints.
    public static func certificateHash(_ der: Data) -> String {
        SHA256.hash(data: der).map { String(format: "%02x", $0) }.joined()
    }

    /// Whether the leaf certificate of `trust` is the pinned one. Compares in
    /// constant time, though the pin is not a secret.
    public static func matches(_ trust: SecTrust, pin: String) -> Bool {
        guard let chain = SecTrustCopyCertificateChain(trust) as? [SecCertificate], let leaf = chain.first else {
            return false
        }
        let got = certificateHash(SecCertificateCopyData(leaf) as Data)
        return constantTimeEqual(Array(got.utf8), Array(pin.lowercased().utf8))
    }

    static func constantTimeEqual(_ a: [UInt8], _ b: [UInt8]) -> Bool {
        guard a.count == b.count else { return false }
        var diff: UInt8 = 0
        for i in a.indices { diff |= a[i] ^ b[i] }
        return diff == 0
    }
}
