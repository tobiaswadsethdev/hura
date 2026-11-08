import AuthenticationServices
import HuraKit
import SwiftUI

/// The Account tab: signing in to hura cloud when signed out, and the
/// account and its devices when signed in. Never in the way: without an
/// account, trackers stay on this phone and everything works as before.
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
                    Text("Your trackers and their filters, kept in one place and the same on every device you sign in on. Optional: without an account, trackers stay on this phone, as they do now.")
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

/// The account, the trackers still on this phone, and the devices.
private struct SignedIn: View {
    @Environment(AccountModel.self) private var account
    @Environment(TrackersModel.self) private var trackers
    @State private var confirmSignOut = false
    @State private var moving = false
    @State private var moveError: String?
    @State private var kept: [String] = []

    var body: some View {
        List {
            Section {
                LabeledContent("Signed in as", value: account.name)
                if let plan = account.account?.plan {
                    LabeledContent("Plan", value: plan.capitalized)
                }
            }
            if !trackers.onPhone.isEmpty {
                Section {
                    ForEach(trackers.onPhone, id: \.source.name) { t in
                        Text(t.source.name)
                    }
                    if moving {
                        ProgressView().frame(maxWidth: .infinity)
                    } else {
                        Button("Move to My Account", systemImage: "icloud.and.arrow.up", action: move)
                    }
                } header: {
                    Text("On this iPhone only")
                } footer: {
                    if !kept.isEmpty {
                        Text("Your account already has a tracker called \(kept.joined(separator: ", ")), so this iPhone's stays here. Forget one of them, here or in the account, to move it.")
                    } else {
                        Text("Added before you signed in, so not read while you are. Moving them sends them and their tokens to hura cloud, which keeps the tokens encrypted, and this iPhone forgets them.")
                    }
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
                Text("This iPhone goes back to the trackers kept on it. Your account keeps its trackers for your other devices.")
            }
            if let error = account.error ?? moveError {
                Section { Text(error).foregroundStyle(.red).font(.callout) }
            }
        }
        .refreshable { await account.load() }
        .confirmationDialog("Sign out of hura cloud?", isPresented: $confirmSignOut, titleVisibility: .visible) {
            Button("Sign Out", role: .destructive) { Task { await account.signOut(trackers: trackers) } }
        }
    }

    private func move() {
        moving = true
        moveError = nil
        Task {
            defer { moving = false }
            do { kept = try await trackers.moveToAccount() } catch { moveError = error.localizedDescription }
        }
    }
}

private struct DeviceRow: View {
    let device: CloudDevice

    var body: some View {
        HStack {
            Image(systemName: icon).foregroundStyle(.secondary).frame(width: 24)
            VStack(alignment: .leading, spacing: 2) {
                Text(device.name)
                Text(device.current ? "This device" : "Last seen \(device.lastSeenAt.formatted(.relative(presentation: .named)))")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private var icon: String {
        switch device.platform {
        case "ios": "iphone"
        case "macos": "laptopcomputer"
        default: "desktopcomputer"
        }
    }
}
