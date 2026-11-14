import HuraKit
import SwiftUI

/// What a pull request's reviewers said, to the agent of the worktree that
/// owns it, as the desktop sends it: by a button in the pull request, and,
/// when that is switched on, for every pull request you opened each time the
/// inbox is read while the app is in front.
///
/// The phone reads the pull request through hura cloud and chooses what to
/// send (`pullRemarks`); the server leaves out what that agent has been sent
/// already, from this phone or the desktop, so asking again says nothing
/// twice.
@MainActor
enum ReviewToAgent {
    /// The switch, as kept in `UserDefaults`. Off by default: it types into an
    /// agent that may be busy with something else.
    static let automaticKey = "inbox.pullCommentsToAgent"

    /// Read the pull request and send what is still open. Answers with the
    /// message, empty when there was nothing new to say.
    static func send(_ stored: StoredTracker, pull: PullRequestSummary, to server: Server,
                     session: String) async throws -> String {
        let detail = try await TrackerCore.pull(stored, repo: pull.repo, id: pull.id)
        let remarks = pullRemarks(detail)
        guard !remarks.isEmpty else { return "" }
        return try await server.api.sendPullComments(session, key: detail.key, remarks: remarks)
    }

    private static var sending = false

    /// The automatic half: every pull request of yours with a worktree on it.
    /// Silent when one fails, as there is nobody to tell; the button says why.
    static func sendAll(_ inbox: Inbox?, model: AppModel, trackers: TrackersModel) {
        guard UserDefaults.standard.bool(forKey: automaticKey), let inbox, !sending else { return }
        let owned = inbox.pulls.filter { $0.relation == "created" }.compactMap { p in
            model.pullSession(p).flatMap { on in trackers.tracker(named: p.tracker).map { (p, on, $0) } }
        }
        guard !owned.isEmpty else { return }
        sending = true
        Task {
            defer { sending = false }
            for (pull, on, stored) in owned {
                _ = try? await send(stored, pull: pull, to: on.server, session: on.name)
            }
        }
    }
}

extension AppModel {
    /// A worktree already on this pull request, on any server: started from
    /// it, or on its branch in its repository.
    func pullSession(_ p: PullRequestSummary) -> (server: Server, name: String)? {
        for server in servers {
            let live = server.sessions.filter { $0.state != "dead" }
            if let s = live.first(where: { $0.pull?.tracker == p.tracker && $0.pull?.repo == p.repo && $0.pull?.id == p.id })
                ?? live.first(where: { $0.workBranch == p.branch && bareRepo($0.repo) == bareRepo(p.repoURL) }) {
                return (server, s.name)
            }
        }
        return nil
    }
}
