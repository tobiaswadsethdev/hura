import Foundation

/// A pairing string from `hurad pair`:
///
///     hura://box.lan:17671/8f3c…e21a#d8fa…e140
///
/// the address, the token, and the hex SHA-256 of the certificate the server
/// will present (`docs/server.md`). There is nothing to redeem: the token in
/// the string is the login, which is why the app checks the server before
/// keeping it.
public struct PairingString: Equatable, Sendable {
    public var host: String
    public var port: String
    public var token: String
    public var fingerprint: String

    public enum ParseError: Error, LocalizedError {
        case notPairingString
        case missingFields

        public var errorDescription: String? {
            switch self {
            case .notPairingString: "That isn't a hura pairing string."
            case .missingFields: "The hura pairing string is incomplete. Copy the whole line `hurad pair` printed."
            }
        }
    }

    public init(host: String, port: String, token: String, fingerprint: String) {
        self.host = host
        self.port = port
        self.token = token
        self.fingerprint = fingerprint
    }

    public static func looksLikeOne(_ raw: String) -> Bool {
        raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased().hasPrefix("hura://")
    }

    public init(_ raw: String) throws {
        guard let c = URLComponents(string: raw.trimmingCharacters(in: .whitespacesAndNewlines)),
              c.scheme == "hura"
        else { throw ParseError.notPairingString }
        let token = c.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        let fingerprint = (c.fragment ?? "").lowercased()
        guard let host = c.host, !host.isEmpty, !token.isEmpty, !token.contains("/"),
              fingerprint.count == 64, fingerprint.allSatisfy(\.isHexDigit)
        else { throw ParseError.missingFields }
        self.host = host.trimmingCharacters(in: CharacterSet(charactersIn: "[]"))
        port = c.port.map(String.init) ?? "17671"
        self.token = token
        self.fingerprint = fingerprint
    }
}

public enum PairingError: Error, LocalizedError, Equatable {
    case notHurad
    case pinMismatch
    case tokenRefused
    case unreachable(String)

    public var errorDescription: String? {
        switch self {
        case .notHurad: "Something answered at that address, but it isn't hurad."
        case .pinMismatch: "The server's certificate doesn't match the pairing string. Nothing was sent."
        case .tokenRefused: "The server doesn't accept this token. Run `hurad pair` again for a new one."
        case let .unreachable(why): "Couldn't reach the server: \(why). Is Tailscale connected?"
        }
    }
}

public enum Pairing {
    /// Checks the server is an `hurad` presenting the pinned certificate and
    /// accepting the token, then returns the record to keep. The token is sent
    /// only after the certificate has matched.
    public static func pair(_ url: PairingString, name: String? = nil) async throws -> ServerRecord {
        let record = ServerRecord(serverID: ServerRecord.newServerID(),
                                name: name.flatMap { $0.isEmpty ? nil : $0 } ?? url.host,
                                host: url.host, port: url.port, fingerprint: url.fingerprint, token: url.token)
        let client = ServerClient(record: record)
        do {
            let hello = try await client.hello()
            guard hello.server == "hurad" else { throw PairingError.notHurad }
            _ = try await client.sessions()
        } catch TransportError.pinMismatch {
            throw PairingError.pinMismatch
        } catch ServerError.unauthorized {
            throw PairingError.tokenRefused
        } catch let e as PairingError {
            throw e
        } catch let e as ServerError {
            // The server answered and took the token; a failure inside it
            // (no sandbox runtime yet, say) is not a reason not to pair.
            if case .failed = e { return record }
            throw PairingError.unreachable(e.localizedDescription)
        } catch {
            throw PairingError.unreachable(error.localizedDescription)
        }
        return record
    }
}

extension String {
    /// Brackets IPv6 literals for use in a URL authority.
    var urlHost: String { contains(":") ? "[\(self)]" : self }
}
