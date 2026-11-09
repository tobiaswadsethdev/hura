import AuthenticationServices
import HuraKit
import SwiftUI

/// The Account tab: signing in to hura cloud when signed out, and the
/// account and its devices when signed in. Servers and sessions need no
/// account; the trackers are kept in one.
struct AccountView: View {
    @Environment(AccountModel.self) private var account
    @Environment(TrackersModel.self) private var trackers

    var body: some View {
        NavigationStack {
            Group {
                if account.session == nil {
                    SignInForm()
                } else {
                    SignedIn()
                }
            }
            .navigationTitle("Account")
        }
        .task { await account.load() }
    }
}

/// An address, then the code emailed to it.
private struct SignInForm: View {
    @Environment(AccountModel.self) private var account
    @Environment(TrackersModel.self) private var trackers
    @State private var email = ""
    @State private var code = ""
    @State private var nonce = ""
    @FocusState private var focused: Bool
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        Form {
            Section {
                if let sentTo = account.codeSentTo {
                    LabeledContent("Sent to", value: sentTo)
                    TextField("Six-digit code", text: $code)
                        .keyboardType(.numberPad)
                        .textContentType(.oneTimeCode)
                        .focused($focused)
                    Button("Sign In", action: verify)
                        .disabled(account.busy || code.filter(\.isNumber).count != 6)
                } else {
                    TextField("you@example.com", text: $email)
                        .keyboardType(.emailAddress)
                        .textContentType(.emailAddress)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .focused($focused)
                        .onSubmit(send)
                    Button("Email Me a Code", action: send)
                        .disabled(account.busy || !email.contains("@"))
                }
            } header: {
                Text("Sign in to hura cloud")
            } footer: {
                if account.codeSentTo != nil {
                    Text("The code works for ten minutes, once.")
                } else {
                    Text("Your trackers and their filters, kept in one place, the same on every device you sign in on, and read all day so you hear about a change on time. Servers and sessions work without an account.")
                }
            }
            if account.codeSentTo == nil {
                Section {
                    SignInWithAppleButton(.signIn) { request in
                        request.requestedScopes = [.email]
                        let n = account.appleNonce()
                        nonce = n.nonce
                        request.nonce = n.hashed
                    } onCompletion: { result in
                        Task { await account.signInWithApple(result, nonce: nonce, trackers: trackers) }
                    }
                    .signInWithAppleButtonStyle(colorScheme == .dark ? .white : .black)
                    .frame(height: 44)
                    .clipShape(Capsule())
                    .listRowInsets(EdgeInsets())
                    .listRowBackground(Color.clear)
                    .disabled(account.busy)
                }
            }
            if account.codeSentTo != nil {
                Section {
                    Button("Send Another Code") { Task { await account.sendCode(to: account.codeSentTo ?? email) } }
                        .disabled(account.busy)
                    Button("Use Another Address") {
                        code = ""
                        account.startOver()
                    }
                }
            }
            if account.busy {
                Section { ProgressView().frame(maxWidth: .infinity) }
            }
            if let error = account.error {
                Section { Text(error).foregroundStyle(.red).font(.callout) }
            }
        }
    }

    private func send() {
        guard email.contains("@") else { return }
        Task {
            await account.sendCode(to: email)
            focused = account.codeSentTo != nil
        }
    }

    private func verify() {
        Task {
            await account.verify(code: code.filter(\.isNumber), trackers: trackers)
            if account.session != nil { code = "" }
        }
    }
}

/// The account and its devices.
private struct SignedIn: View {
    @Environment(AccountModel.self) private var account
    @Environment(TrackersModel.self) private var trackers
    @State private var confirmSignOut = false

    var body: some View {
        List {
            Section {
                LabeledContent("Signed in as", value: account.name)
                if let plan = account.account?.plan {
                    LabeledContent("Plan", value: plan.capitalized)
                }
            }
            Section("Devices") {
                ForEach(account.devices) { d in
                    DeviceRow(device: d)
                        .swipeActions {
                            if !d.current {
                                Button("Sign Out", role: .destructive) { Task { await account.signOut(device: d) } }
                            }
                        }
                }
            }
            Section {
                Button("Sign Out", role: .destructive) { confirmSignOut = true }
            } footer: {
                Text("This iPhone stops showing your trackers. Your account keeps them for your other devices.")
            }
            if let error = account.error {
                Section { Text(error).foregroundStyle(.red).font(.callout) }
            }
        }
        .refreshable { await account.load() }
        .alert("Sign out of hura cloud?", isPresented: $confirmSignOut) {
            Button("Cancel", role: .cancel) {}
            Button("Sign Out", role: .destructive) { Task { await account.signOut(trackers: trackers) } }
        } message: {
            Text("This iPhone stops showing your trackers until you sign in again.")
        }
    }
}

private struct DeviceRow: View {
    let device: CloudDevice

    var body: some View {
        HStack {
            Image(glyph: icon).foregroundStyle(.secondary).frame(width: 24)
            VStack(alignment: .leading, spacing: 2) {
                Text(device.name)
                Text(device.current ? "This device" : "Last seen \(device.lastSeenAt.formatted(.relative(presentation: .named)))")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private var icon: Glyph {
        switch device.platform {
        case "ios": .phone
        case "macos": .laptop
        default: .desktop
        }
    }
}
