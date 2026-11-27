import Foundation

/// Keeping a server's list to what needs you, T3 Code's way, as the desktop
/// does (`apps/desktop/src/triage.ts`): a worktree can be pinned to the top,
/// settled onto a shelf at the bottom once its work is done, or snoozed until
/// a time, when it comes back by itself.
///
/// The phone's own, not the server's: kept in this phone's defaults, per
/// server and by session name, so pinning something here pins nothing on the
/// desktop. A worktree that is waiting on you is never kept out of sight by
/// any of it: a settled or snoozed one that asks something comes back to its
/// project.
struct Triage: Codable, Equatable {
    var pinned = false
    /// When it was settled.
    var settled: Date?
    /// When it comes back.
    var snoozedUntil: Date?

    var isEmpty: Bool { !pinned && settled == nil && snoozedUntil == nil }

    /// What can be done to a worktree's triage.
    enum Change: Equatable {
        case pin, unpin
        case settle(Date), unsettle
        case snooze(until: Date), wake
    }

    /// Where a worktree is drawn.
    enum Place: Equatable {
        case pinned, active, snoozed, settled
    }

    /// Settling unpins and wakes it, since it is done; snoozing unsettles it,
    /// since it is coming back; pinning wakes and unsettles it, since it is
    /// wanted now. The same rules as the desktop's.
    mutating func apply(_ change: Change) {
        switch change {
        case .pin:
            pinned = true
            settled = nil
            snoozedUntil = nil
        case .unpin:
            pinned = false
        case let .settle(at):
            settled = at
            pinned = false
            snoozedUntil = nil
        case .unsettle:
            settled = nil
        case let .snooze(until):
            snoozedUntil = until
            settled = nil
        case .wake:
            snoozedUntil = nil
        }
    }

    /// Where it goes at `now`. One that needs you stays in sight, pinned if
    /// it was.
    func place(needsYou: Bool, now: Date) -> Place {
        if let until = snoozedUntil, until > now, !needsYou { return .snoozed }
        if settled != nil, !needsYou { return .settled }
        return pinned ? .pinned : .active
    }

    /// The times a worktree can be snoozed until, from `now`: an hour, this
    /// evening (six o'clock, while it is still before five), tomorrow morning
    /// and next Monday morning (nine).
    static func snoozeChoices(now: Date = Date(), calendar: Calendar = .current) -> [(label: String, until: Date)] {
        func at(days: Int, hour: Int) -> Date {
            let day = calendar.date(byAdding: .day, value: days, to: calendar.startOfDay(for: now)) ?? now
            return calendar.date(bySettingHour: hour, minute: 0, second: 0, of: day) ?? day
        }
        // Sunday is 1 to a calendar; the days to the coming Monday, a whole
        // week on a Monday.
        let weekday = calendar.component(.weekday, from: now) - 1
        let monday = (8 - weekday) % 7 == 0 ? 7 : (8 - weekday) % 7
        var choices: [(label: String, until: Date)] = [("For an Hour", now.addingTimeInterval(3600))]
        if calendar.component(.hour, from: now) < 17 {
            choices.append(("Until This Evening", at(days: 0, hour: 18)))
        }
        choices.append(("Until Tomorrow Morning", at(days: 1, hour: 9)))
        choices.append(("Until Next Week", at(days: monday, hour: 9)))
        return choices
    }
}

/// One server's triage, by session name, and where it is kept.
struct TriageBook: Equatable {
    private(set) var entries: [String: Triage] = [:]

    subscript(name: String) -> Triage { entries[name] ?? Triage() }

    /// Changes one, forgetting it once there is nothing left to remember.
    mutating func apply(_ change: Triage.Change, to name: String) {
        var t = self[name]
        t.apply(change)
        entries[name] = t.isEmpty ? nil : t
    }

    /// Forgets one: a worktree that has been deleted.
    mutating func forget(_ name: String) {
        entries[name] = nil
    }

    /// The soonest a snoozed worktree comes back after `now`, when the list
    /// has to be looked at again.
    func nextWake(after now: Date) -> Date? {
        entries.values.compactMap(\.snoozedUntil).filter { $0 > now }.min()
    }

    private static func key(_ server: String) -> String { "hura.triage.\(server)" }

    static func load(server: String, from defaults: UserDefaults = .standard) -> TriageBook {
        guard let data = defaults.data(forKey: key(server)),
              let entries = try? JSONDecoder().decode([String: Triage].self, from: data)
        else { return TriageBook() }
        return TriageBook(entries: entries)
    }

    func save(server: String, to defaults: UserDefaults = .standard) {
        // The list works without it; it just will not remember.
        guard let data = try? JSONEncoder().encode(entries) else { return }
        defaults.set(data, forKey: Self.key(server))
    }
}
