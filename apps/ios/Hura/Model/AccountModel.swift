import HuraKit
import Observation
import UIKit

/// This phone and hura cloud: signing in with an emailed code, the account
/// and the devices signed in to it, and signing out. Optional: nothing else
/// in the app waits on it, and signed out, everything works as it always did.
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

    /// Signs in with the code, then reads the account's trackers in place of
    /// this phone's.
    func verify(code: String, trackers: TrackersModel) async {
        guard let email = codeSentTo else { return }
        await working {
            try await cloud.verify(email: email, code: code, deviceName: UIDevice.current.name)
            codeSentTo = nil
        }
        await load()
        await trackers.refresh()
    }

    /// Signs this phone out, and goes back to the trackers kept on it.
    func signOut(trackers: TrackersModel) async {
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
