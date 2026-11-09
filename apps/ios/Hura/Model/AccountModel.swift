import AuthenticationServices
import CryptoKit
import HuraKit
import Observation
import UIKit

/// This phone and hura cloud: signing in with an emailed code, the account
/// and the devices signed in to it, and signing out. Optional for servers and
/// sessions; the trackers are the account's, so signed out there are none.
@MainActor
@Observable
final class AccountModel {
    private(set) var session: CloudSession?
    private(set) var account: CloudAccount?
    private(set) var devices: [CloudDevice] = []
    /// Where the code was sent, while one is waiting to be typed in.
    private(set) var codeSentTo: String?
    private(set) var busy = false
    var error: String?

    @ObservationIgnored private let cloud: Cloud

    init(cloud: Cloud = .shared) {
        self.cloud = cloud
        // The session kept from the last launch, read at once, so the
        // Account tab opens on the account rather than flashing the sign-in
        // form while `load` asks.
        session = cloud.stored
    }

    /// The session as it stands, and when signed in, the account and its
    /// devices. A phone signed out from elsewhere finds out here.
    func load() async {
        session = await cloud.current
        guard session != nil else {
            account = nil
            devices = []
            return
        }
        do {
            account = try await cloud.account()
            devices = try await cloud.devices()
        } catch {
            fail(error)
            session = await cloud.current
        }
    }

    func sendCode(to email: String) async {
        let email = email.trimmingCharacters(in: .whitespaces)
        await working {
            try await cloud.sendCode(to: email)
            codeSentTo = email
        }
    }

    /// Back to typing an address.
    func startOver() {
        codeSentTo = nil
        error = nil
    }

    /// Signs in with the code, then reads the account's trackers.
    func verify(code: String, trackers: TrackersModel) async {
        guard let email = codeSentTo else { return }
        await working {
            try await cloud.verify(email: email, code: code, deviceName: UIDevice.current.name)
            codeSentTo = nil
        }
        await load()
        await trackers.refresh()
    }

    /// A fresh nonce for one Sign in with Apple: its SHA-256 goes to Apple in
    /// the request, and the nonce itself to the service with the token, which
    /// ties the token to this sign-in.
    func appleNonce() -> (nonce: String, hashed: String) {
        let nonce = (0..<32).map { _ in String(format: "%02x", UInt8.random(in: 0...255)) }.joined()
        let hashed = SHA256.hash(data: Data(nonce.utf8)).map { String(format: "%02x", $0) }.joined()
        return (nonce, hashed)
    }

    /// Signs in with what Sign in with Apple answered. Cancelled is not a
    /// failure: the person changed their mind.
    func signInWithApple(_ result: Result<ASAuthorization, Error>, nonce: String, trackers: TrackersModel) async {
        let credential: ASAuthorizationAppleIDCredential
        switch result {
        case let .success(authorization):
            guard let c = authorization.credential as? ASAuthorizationAppleIDCredential,
                  let token = c.identityToken.flatMap({ String(data: $0, encoding: .utf8) })
            else {
                error = "Apple didn't hand over a sign-in to check."
                return
            }
            credential = c
            await working {
                try await cloud.signInWithApple(identityToken: token, nonce: nonce, email: credential.email,
                                                deviceName: UIDevice.current.name)
                codeSentTo = nil
            }
        case let .failure(e):
            if (e as? ASAuthorizationError)?.code != .canceled { error = e.localizedDescription }
            return
        }
        await load()
        await trackers.refresh()
    }

    /// What the account is known by: the address this phone signed in with,
    /// or one the account has, or that it is an Apple ID's.
    var name: String {
        if let email = session?.email, !email.isEmpty { return email }
        return account?.signIns.compactMap(\.email).first ?? "Apple ID"
    }

    /// Signs this phone out, which leaves it with no trackers.
    func signOut(trackers: TrackersModel) async {
        await trackers.leaveCloud()
        await working { await cloud.signOut() }
        session = nil
        account = nil
        devices = []
        await trackers.refresh()
    }

    /// Signs another device out; its tokens stop working at once.
    func signOut(device: CloudDevice) async {
        await working {
            try await cloud.signOut(device: device.id)
            devices.removeAll { $0.id == device.id }
        }
    }

    private func working(_ body: () async throws -> Void) async {
        busy = true
        error = nil
        defer { busy = false }
        do { try await body() } catch { fail(error) }
    }

    private func fail(_ error: Error) {
        self.error = error.localizedDescription
    }
}
