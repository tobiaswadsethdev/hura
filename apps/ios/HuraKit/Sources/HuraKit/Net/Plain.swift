import Foundation
import Network

// Failures in words.
//
// Every error the app shows was, until this, its `localizedDescription`, and
// for the errors that come from the system that is the system talking to a
// developer: "The operation couldn't be completed. (Network.NWError error
// 61 - Connection refused)", "The data couldn't be read because it is missing."
// What a failure means *here* is narrower than what the system can say, so
// it is said here, once, for every screen that shows one.

enum Plain {
    /// Why a connection to a server failed, from what Network.framework said.
    static func network(_ error: NWError?) -> String {
        switch error {
        case .none:
            return "The connection closed."
        case let .posix(code)?:
            switch code {
            case .ECONNREFUSED:
                return "Nothing is listening at that address. Is hurad running?"
            case .ETIMEDOUT:
                return "The server didn't answer in time."
            case .ENETUNREACH, .EHOSTUNREACH, .ENETDOWN, .EHOSTDOWN:
                return "This iPhone can't reach the server. Check it's on the same network or VPN."
            case .ECONNRESET, .ECONNABORTED, .EPIPE, .ENOTCONN:
                return "The connection to the server was cut off."
            case .ECANCELED:
                return "The connection was cancelled."
            case .EADDRINUSE:
                return "That port is already in use on this iPhone."
            default:
                return "Couldn't connect to the server."
            }
        case .dns?:
            return "Couldn't find the server's address. Check the network."
        case .tls?:
            return "The secure connection to the server failed."
        default:
            return "Couldn't connect to the server."
        }
    }

    /// An HTTP status from a hurad that was not the answer.
    static func status(_ code: Int) -> String {
        switch code {
        case 401, 403: "The server no longer accepts this phone. Pair it again with `hurad pair`."
        case 404: "The server doesn't know that request. It may need updating."
        case 500...599: "The server ran into a problem answering (HTTP \(code))."
        default: "The server refused the request (HTTP \(code))."
        }
    }

    /// A message written as a clause by the server -- `no session named x` --
    /// started like the sentence it is once shown on its own. Not when the
    /// first word is hura's own, which is lowercase everywhere.
    static func sentence(_ message: String) -> String {
        guard let first = message.first, first.isLowercase, first.isASCII, !message.hasPrefix("hura")
        else { return message }
        return first.uppercased() + message.dropFirst()
    }
}

public extension Error {
    /// What to show a person about this error.
    ///
    /// The app's own errors already say it in their `errorDescription`; what
    /// this adds is words for the system's, which would otherwise surface as
    /// a domain and a code.
    var userMessage: String {
        switch self {
        case let e as NWError:
            return Plain.network(e)
        case is DecodingError:
            return "The answer couldn't be read. hura may need updating."
        case let e as URLError:
            switch e.code {
            case .notConnectedToInternet, .networkConnectionLost, .dataNotAllowed:
                return "This iPhone is offline."
            case .timedOut:
                return "hura cloud didn't answer in time."
            case .cannotFindHost, .cannotConnectToHost, .dnsLookupFailed:
                return "Couldn't reach hura cloud. Check the network."
            case .cancelled:
                return "The request was cancelled."
            default:
                return "Couldn't reach hura cloud."
            }
        default:
            return Plain.sentence(localizedDescription)
        }
    }
}
