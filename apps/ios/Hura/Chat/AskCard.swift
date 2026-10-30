import HuraKit
import SwiftUI

/// What a conversation waits on you for, as something to answer. Claude Code
/// asks three kinds of thing through one callback: a tool it wants to run
/// (allowed, allowed from now on, or declined with what to do instead), a
/// question with choices, and a plan to approve into a mode or send back.
/// The same three the desktop window draws (`chat/Ask.tsx`).
struct AskCard: View {
    let ask: Ask
    let enabled: Bool
    let onAnswer: (ChatDecision) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            switch ask.tool {
            case "AskUserQuestion": QuestionsForm(ask: ask, onAnswer: onAnswer)
            case "ExitPlanMode": PlanForm(ask: ask, onAnswer: onAnswer)
            default: PermissionForm(ask: ask, onAnswer: onAnswer)
            }
        }
        .disabled(!enabled)
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.orange.opacity(0.15))
    }
}

private struct PermissionForm: View {
    let ask: Ask
    let onAnswer: (ChatDecision) -> Void
    @State private var declining = false
    @State private var message = ""
    @FocusState private var typing: Bool

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "hand.raised.fill").foregroundStyle(.orange)
            VStack(alignment: .leading, spacing: 4) {
                Text(ask.summary).font(.subheadline.weight(.semibold))
                if let d = ask.description, !d.isEmpty, !ask.summary.contains(d) {
                    Text(d).font(.caption).foregroundStyle(.secondary)
                }
                if let r = ask.reason { Text(r).font(.caption).foregroundStyle(.secondary) }
                if let p = ask.blockedPath { Text("Outside the working copy: \(p)").font(.caption).foregroundStyle(.secondary) }
                if ask.agent != nil { Text("Asked by a subagent").font(.caption).foregroundStyle(.secondary) }
            }
        }
        preview
        if declining {
            HStack {
                TextField("What to do instead (optional)", text: $message)
                    .textFieldStyle(.roundedBorder)
                    .focused($typing)
                    .onAppear { typing = true }
                    .submitLabel(.send)
                    .onSubmit(decline)
                Button("Decline", role: .destructive, action: decline)
                    .buttonStyle(.borderedProminent)
                    .tint(.red)
            }
            Button("Back") { declining = false }.font(.caption)
        } else {
            HStack {
                Button {
                    onAnswer(ChatDecision(allow: true))
                } label: {
                    Label("Allow", systemImage: "checkmark").frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                Button(role: .destructive) {
                    declining = true
                } label: {
                    Label("Decline…", systemImage: "xmark")
                        .foregroundStyle(.red)
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                .tint(.red)
            }
            if ask.canRemember {
                Button("Allow, and don't ask again") {
                    onAnswer(ChatDecision(allow: true, always: true))
                }
                .font(.footnote)
            }
        }
    }

    @ViewBuilder
    private var preview: some View {
        let text = { (key: String) in ask.input[key]?.string ?? "" }
        switch ask.tool {
        case "Bash":
            code(text("command"))
        case "Edit":
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(text("old_string").split(separator: "\n", omittingEmptySubsequences: false).prefix(20).enumerated()), id: \.offset) {
                    Text("- " + $0.element).foregroundStyle(.red)
                }
                ForEach(Array(text("new_string").split(separator: "\n", omittingEmptySubsequences: false).prefix(20).enumerated()), id: \.offset) {
                    Text("+ " + $0.element).foregroundStyle(.green)
                }
            }
            .font(.caption.monospaced())
            .padding(8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.black.opacity(0.4), in: RoundedRectangle(cornerRadius: 6))
        case "Write":
            code(text("content").split(separator: "\n", omittingEmptySubsequences: false).prefix(20).joined(separator: "\n"))
        default:
            EmptyView()
        }
    }

    private func code(_ s: String) -> some View {
        ScrollView(.horizontal) {
            Text(s).font(.caption.monospaced()).textSelection(.enabled).padding(8)
        }
        .frame(maxHeight: 160)
        .background(Color.black.opacity(0.4), in: RoundedRectangle(cornerRadius: 6))
    }

    private func decline() {
        let m = message.trimmingCharacters(in: .whitespacesAndNewlines)
        onAnswer(ChatDecision(allow: false, message: m.isEmpty ? nil : m))
    }
}

private struct QuestionsForm: View {
    let ask: Ask
    let onAnswer: (ChatDecision) -> Void
    @State private var chosen: [String: [String]] = [:]
    @State private var other: [String: String] = [:]

    private struct Question {
        var question: String
        var header: String?
        var options: [(label: String, description: String?)]
        var multiSelect: Bool
    }

    private var questions: [Question] {
        guard case let .array(list)? = ask.input["questions"] else { return [] }
        return list.map { q in
            let options: [(label: String, description: String?)]
            if case let .array(os)? = q["options"] {
                options = os.map { (label: $0["label"]?.string ?? "", description: $0["description"]?.string) }
            } else {
                options = []
            }
            return Question(question: q["question"]?.string ?? "", header: q["header"]?.string, options: options,
                            multiSelect: q["multiSelect"]?.bool ?? false)
        }
    }

    private func answer(for q: String) -> String {
        let typed = (other[q] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        return typed.isEmpty ? (chosen[q] ?? []).joined(separator: ", ") : typed
    }

    var body: some View {
        let qs = questions
        Label("Claude has a question", systemImage: "questionmark.bubble.fill")
            .font(.subheadline.weight(.semibold))
            .foregroundStyle(.orange)
        ForEach(Array(qs.enumerated()), id: \.offset) { _, q in
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .firstTextBaseline) {
                    if let h = q.header {
                        Text(h).font(.caption2.weight(.semibold)).padding(.horizontal, 6).padding(.vertical, 2)
                            .background(Color.white.opacity(0.1), in: Capsule())
                    }
                    Text(q.question).font(.subheadline)
                }
                ForEach(Array(q.options.enumerated()), id: \.offset) { _, o in
                    let on = (chosen[q.question] ?? []).contains(o.label)
                    Button {
                        toggle(q, o.label)
                    } label: {
                        HStack(alignment: .top) {
                            Image(systemName: on ? (q.multiSelect ? "checkmark.square.fill" : "largecircle.fill.circle")
                                                 : (q.multiSelect ? "square" : "circle"))
                            VStack(alignment: .leading, spacing: 2) {
                                Text(o.label).font(.callout.weight(.medium))
                                if let d = o.description { Text(d).font(.caption).foregroundStyle(.secondary) }
                            }
                            Spacer(minLength: 0)
                        }
                        .padding(8)
                        .background(on ? Color.accentColor.opacity(0.2) : Color.white.opacity(0.05),
                                    in: RoundedRectangle(cornerRadius: 8))
                    }
                    .buttonStyle(.plain)
                }
                TextField("Or say something else", text: Binding(
                    get: { other[q.question] ?? "" },
                    set: { other[q.question] = $0; if !$0.isEmpty { chosen[q.question] = [] } }))
                    .textFieldStyle(.roundedBorder)
                    .font(.callout)
            }
        }
        HStack {
            Button {
                onAnswer(ChatDecision(allow: true, answers: Dictionary(qs.map { ($0.question, answer(for: $0.question)) },
                                                                       uniquingKeysWith: { a, _ in a })))
            } label: {
                Text("Answer").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .disabled(!qs.allSatisfy { !answer(for: $0.question).isEmpty })
            Button("Skip") {
                onAnswer(ChatDecision(allow: false, message: "The user did not answer."))
            }
            .buttonStyle(.bordered)
        }
    }

    private func toggle(_ q: Question, _ label: String) {
        other[q.question] = ""
        let now = chosen[q.question] ?? []
        if !q.multiSelect {
            chosen[q.question] = [label]
        } else {
            chosen[q.question] = now.contains(label) ? now.filter { $0 != label } : now + [label]
        }
    }
}

private struct PlanForm: View {
    let ask: Ask
    let onAnswer: (ChatDecision) -> Void
    @State private var revising = false
    @State private var message = ""
    @State private var expanded = false
    @FocusState private var typing: Bool

    var body: some View {
        Label("Claude has a plan", systemImage: "list.bullet.clipboard.fill")
            .font(.subheadline.weight(.semibold))
            .foregroundStyle(.orange)
        ScrollView {
            MarkdownText(text: ask.input["plan"]?.string ?? "")
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(maxHeight: expanded ? 520 : 220)
        Button(expanded ? "Show less" : "Show more") { withAnimation { expanded.toggle() } }
            .font(.caption)
        if revising {
            HStack {
                TextField("What to change", text: $message)
                    .textFieldStyle(.roundedBorder)
                    .focused($typing)
                    .onAppear { typing = true }
                    .submitLabel(.send)
                    .onSubmit(sendBack)
                Button("Send Back", action: sendBack).buttonStyle(.borderedProminent)
            }
            Button("Back") { revising = false }.font(.caption)
        } else {
            Button {
                onAnswer(ChatDecision(allow: true, mode: "acceptEdits"))
            } label: {
                Text("Approve, and accept edits").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            HStack {
                Button {
                    onAnswer(ChatDecision(allow: true, mode: "default"))
                } label: {
                    Text("Approve, ask for each edit").frame(maxWidth: .infinity)
                }
                Button {
                    revising = true
                } label: {
                    Text("Keep planning…").frame(maxWidth: .infinity)
                }
            }
            .buttonStyle(.bordered)
            .font(.footnote)
        }
    }

    private func sendBack() {
        let m = message.trimmingCharacters(in: .whitespacesAndNewlines)
        onAnswer(ChatDecision(allow: false, message: m.isEmpty ? "Keep planning." : m))
    }
}
