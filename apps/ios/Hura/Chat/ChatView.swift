import HuraKit
import SwiftUI

/// A chat session's conversation: the transcript, the reply as it is
/// written, what is waiting on you, and a composer. hura runs Claude Code
/// through the Agent SDK, so a permission request is a request with an id,
/// answered with a button.
struct ChatView: View {
    let chat: Chat
    @State private var draft = ""
    @State private var sending = false
    @State private var answering: String?
    @State private var atBottom = true
    @State private var items: [ChatItem] = []
    @State private var scrolledToLatest = false
    @FocusState private var composing: Bool

    /// hura clones every session's repository here.
    private static let cwd = "/sandbox/repo"
    private var working: Bool { chat.activity == "running" }

    var body: some View {
        VStack(spacing: 0) {
            if case let .offline(why) = chat.phase {
                Label(why, systemImage: "wifi.slash")
                    .font(.footnote)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(10)
                    .background(Color.gray.opacity(0.25))
            }
            ScrollViewReader { proxy in
                ZStack(alignment: .bottomTrailing) {
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 12) {
                            if chat.entries.isEmpty { emptyState }
                            ChatRows(items: items, cwd: Self.cwd, working: working)
                            if !chat.streaming.isEmpty {
                                MarkdownText(text: chat.streaming).opacity(0.8)
                            }
                            if working && chat.streaming.isEmpty && chat.pending.isEmpty {
                                ProgressView().frame(maxWidth: .infinity, alignment: .leading)
                            }
                            Color.clear.frame(height: 1).id("bottom")
                                .onAppear { atBottom = true }
                                .onDisappear { atBottom = false }
                        }
                        .padding(.horizontal, 14)
                        .padding(.vertical, 12)
                    }
                    .onChange(of: chat.entries.count, initial: true) {
                        items = ChatItem.fold(chat.entries)
                        if !scrolledToLatest, !items.isEmpty {
                            scrolledToLatest = true
                            proxy.scrollTo("bottom", anchor: .bottom)
                        } else if atBottom {
                            withAnimation { proxy.scrollTo("bottom", anchor: .bottom) }
                        }
                    }
                    .onChange(of: chat.streaming) {
                        if atBottom { proxy.scrollTo("bottom", anchor: .bottom) }
                    }
                    .scrollDismissesKeyboard(.interactively)
                    .defaultScrollAnchor(.bottom)
                    if !atBottom {
                        Button {
                            withAnimation { proxy.scrollTo("bottom", anchor: .bottom) }
                        } label: {
                            Image(systemName: "arrow.down")
                                .font(.headline)
                                .padding(10)
                                .background(.ultraThinMaterial, in: Circle())
                        }
                        .padding(14)
                    }
                }
            }
            ForEach(chat.pending) { ask in
                askBanner(ask)
            }
            composer
        }
        .background(Color.black)
        // Dark whatever the system setting: the rows and tool cards are drawn
        // for a dark background.
        .environment(\.colorScheme, .dark)
        .alert("Couldn't do that", isPresented: Binding(get: { chat.error != nil }, set: { if !$0 { chat.error = nil } })) {
            Button("OK") { chat.error = nil }
        } message: {
            Text(chat.error ?? "")
        }
    }

    @ViewBuilder
    private var emptyState: some View {
        VStack(spacing: 8) {
            if let notice = chat.notice {
                Text(notice)
            } else if chat.loaded {
                Text("No messages yet").font(.headline)
                Text("Send Claude a message below.")
            } else {
                ProgressView()
            }
        }
        .font(.subheadline)
        .foregroundStyle(.secondary)
        .multilineTextAlignment(.center)
        .frame(maxWidth: .infinity)
        .padding(.top, 60)
    }

    private func askBanner(_ ask: Ask) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "hand.raised.fill").foregroundStyle(.orange)
                VStack(alignment: .leading, spacing: 4) {
                    Text(ask.summary)
                        .font(.subheadline.monospaced())
                        .textSelection(.enabled)
                    if let d = ask.description, !d.isEmpty {
                        Text(d).font(.caption).foregroundStyle(.secondary)
                    }
                }
                Spacer(minLength: 0)
            }
            if ask.isQuestion {
                Text("This needs a choice; answer it in hura's desktop window.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                HStack {
                    Button {
                        answer(ask, allow: true)
                    } label: {
                        Label("Allow", systemImage: "checkmark").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    Button(role: .destructive) {
                        answer(ask, allow: false)
                    } label: {
                        Label("Deny", systemImage: "xmark")
                            .foregroundStyle(.red)
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                    .tint(.red)
                }
                .disabled(answering != nil || chat.phase != .live)
            }
        }
        .padding(12)
        .background(Color.orange.opacity(0.15))
    }

    private var composer: some View {
        HStack(alignment: .bottom, spacing: 8) {
            TextField("Message Claude", text: $draft, axis: .vertical)
                .lineLimit(1...6)
                .focused($composing)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 18))
            if working && draft.isEmpty {
                Button {
                    Task { try? await chat.interrupt() }
                } label: {
                    Image(systemName: "stop.circle.fill").font(.system(size: 30))
                }
                .tint(.red)
                .accessibilityLabel("Stop Claude")
            } else {
                Button(action: send) {
                    Image(systemName: "arrow.up.circle.fill").font(.system(size: 30))
                }
                .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || sending)
                .accessibilityLabel("Send")
            }
        }
        .disabled(chat.phase != .live)
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(.bar)
    }

    private func send() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        sending = true
        Task {
            defer { sending = false }
            do {
                try await chat.send(text)
                draft = ""
            } catch {
                chat.error = error.localizedDescription
            }
        }
    }

    private func answer(_ ask: Ask, allow: Bool) {
        answering = ask.id
        Task {
            defer { answering = nil }
            do { try await chat.answer(ask.id, allow: allow) } catch { chat.error = error.localizedDescription }
        }
    }
}
