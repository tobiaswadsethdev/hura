import SwiftUI

/// hura's own icons: Lucide, the set the desktop draws in, so a branch, a
/// ticket or a sandbox's policy is the same mark on both. Named for what they
/// mean, as `apps/desktop/src/icons.tsx` names them, with the Lucide icon each
/// is drawn as.
///
/// Standard iOS controls keep Apple's symbols -- the back chevron, the ••• of a
/// menu, share, close, copy, a checkmark in a list -- because those are the
/// platform's, and look like every other app's on purpose.
///
/// Each is a custom symbol in `Assets.xcassets/Lucide`, made from Lucide's SVG
/// by `Tools/lucide-symbols.sh`, which reads the list below: add a case here,
/// run the script, and the symbol is there. As symbols rather than plain
/// images they size with the text beside them and take its weight, as SF
/// Symbols do.
enum Glyph: String, CaseIterable {
    // Places
    case server = "server"
    case inbox = "inbox"
    case account = "circle-user-round"
    case settings = "settings"
    case integrations = "plug"
    case info = "info"
    case dock = "panel-right"

    // Projects, worktrees and git
    case folder = "folder"
    case files = "folder-tree"
    case newProject = "folder-plus"
    case forgetProject = "folder-minus"
    case branch = "git-branch"
    case merge = "git-merge"
    case commit = "git-commit-horizontal"
    case diff = "file-diff"
    case fetch = "cloud-download"
    case pull = "arrow-down-to-line"
    case publish = "cloud-upload"
    case push = "arrow-up-from-line"

    // Files
    case file = "file"
    case fileText = "file-text"
    case fileConfig = "file-cog"
    case fileImage = "images"
    case fileNew = "file-plus"
    case binary = "binary"

    // The sandbox
    case events = "activity"
    case policy = "shield"
    case allow = "shield-check"
    case block = "shield-ban"
    case unsandboxed = "shield-off"
    case ports = "globe"
    case stopForwarding = "circle-stop"
    case kill = "octagon-x"
    case start = "play"
    case stop = "square"
    case restart = "rotate-cw"

    // An agent and what it is doing
    case chat = "messages-square"
    case newConversation = "message-square-plus"
    case agentTerminal = "terminal"
    case shell = "square-terminal"
    case waiting = "circle-question-mark"
    case failed = "circle-alert"
    case permission = "hand"
    case question = "message-circle-question-mark"
    case plan = "clipboard-list"
    case todos = "list-checks"
    case model = "cpu"
    case effort = "brain"
    case agent = "bot"
    case tool = "wrench"
    case edit = "pencil"
    case search = "search"
    case web = "earth"
    case reply = "reply"
    case todo = "circle"
    case doing = "circle-dot-dashed"

    // Messages
    case sent = "check"
    case read = "check-check"
    case offline = "wifi-off"
    case closed = "circle-off"

    // Tickets and pull requests
    case tracker = "tag"
    case pullRequest = "git-pull-request"
    case comment = "message-square"
    /// Somebody else had the last word.
    case commentNew = "message-square-dot"
    case reviewComment = "message-square-text"
    case sendReview = "send"
    case workOn = "hammer"
    case review = "eye"
    case stopWatching = "eye-off"
    case approve = "badge-check"
    case autoMerge = "clock"
    case goTo = "circle-arrow-right"
    case passing = "circle-check"
    case failing = "circle-x"
    case pending = "circle-dashed"
    case skipped = "circle-minus"
    case warning = "triangle-alert"

    // Devices
    case phone = "smartphone"
    case laptop = "laptop"
    case desktop = "monitor"

    /// The asset's name.
    var name: String { "lucide.\(rawValue)" }
}

extension Image {
    init(glyph: Glyph) { self.init(glyph.name) }
}

extension Label where Title == Text, Icon == Image {
    init(_ title: some StringProtocol, glyph: Glyph) {
        self.init(title, image: glyph.name)
    }
}

extension Button where Label == SwiftUI.Label<Text, Image> {
    init(_ title: some StringProtocol, glyph: Glyph, role: ButtonRole? = nil, action: @escaping () -> Void) {
        self.init(role: role, action: action) { SwiftUI.Label(title, glyph: glyph) }
    }
}
