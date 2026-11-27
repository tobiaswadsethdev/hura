import Foundation
import Testing
@testable import Hura

/// A calendar on a fixed zone, so the snooze times are the same wherever the
/// tests run.
private var calendar: Calendar {
    var c = Calendar(identifier: .gregorian)
    c.timeZone = TimeZone(identifier: "Europe/Stockholm")!
    return c
}

private func date(_ y: Int, _ m: Int, _ d: Int, _ h: Int, _ min: Int = 0) -> Date {
    calendar.date(from: DateComponents(year: y, month: m, day: d, hour: h, minute: min))!
}

@Test func settlingUnpinsAndWakes() {
    var t = Triage()
    t.apply(.pin)
    t.apply(.snooze(until: date(2026, 11, 27, 18)))
    #expect(t.pinned)
    t.apply(.settle(date(2026, 11, 27, 10)))
    #expect(!t.pinned)
    #expect(t.snoozedUntil == nil)
    #expect(t.settled == date(2026, 11, 27, 10))
}

@Test func snoozingUnsettles() {
    var t = Triage()
    t.apply(.settle(date(2026, 11, 27, 10)))
    t.apply(.snooze(until: date(2026, 11, 27, 18)))
    #expect(t.settled == nil)
    #expect(t.snoozedUntil == date(2026, 11, 27, 18))
}

@Test func pinningWakesAndUnsettles() {
    var t = Triage()
    t.apply(.snooze(until: date(2026, 11, 27, 18)))
    t.apply(.pin)
    #expect(t.pinned && t.snoozedUntil == nil)
    t.apply(.settle(date(2026, 11, 27, 10)))
    t.apply(.pin)
    #expect(t.pinned && t.settled == nil)
}

@Test func aWorktreeGoesWhereItsTriageSays() {
    let now = date(2026, 11, 27, 12)
    var t = Triage()
    #expect(t.place(needsYou: false, now: now) == .active)
    t.apply(.pin)
    #expect(t.place(needsYou: false, now: now) == .pinned)
    t.apply(.snooze(until: date(2026, 11, 27, 13)))
    #expect(t.place(needsYou: false, now: now) == .snoozed)
    // Back by itself once the time has come, still pinned: snoozing only
    // unsettles.
    #expect(t.place(needsYou: false, now: date(2026, 11, 27, 13, 1)) == .pinned)
    t.apply(.settle(now))
    #expect(t.place(needsYou: false, now: now) == .settled)
}

@Test func oneWaitingOnYouIsNeverPutAway() {
    let now = date(2026, 11, 27, 12)
    var t = Triage()
    t.apply(.snooze(until: date(2026, 11, 28, 9)))
    #expect(t.place(needsYou: true, now: now) == .active)
    t.apply(.settle(now))
    #expect(t.place(needsYou: true, now: now) == .active)
}

@Test func aBookForgetsWhatIsLeftEmpty() {
    var book = TriageBook()
    book.apply(.pin, to: "fix-tests")
    #expect(book["fix-tests"].pinned)
    book.apply(.unpin, to: "fix-tests")
    #expect(book.entries.isEmpty)
    book.apply(.snooze(until: date(2026, 11, 27, 18)), to: "a")
    book.apply(.snooze(until: date(2026, 11, 27, 14)), to: "b")
    #expect(book.nextWake(after: date(2026, 11, 27, 12)) == date(2026, 11, 27, 14))
    #expect(book.nextWake(after: date(2026, 11, 27, 15)) == date(2026, 11, 27, 18))
    #expect(book.nextWake(after: date(2026, 11, 27, 19)) == nil)
}

@Test func aBookIsKeptPerServer() throws {
    let defaults = try #require(UserDefaults(suiteName: "hura.tests.\(UUID().uuidString)"))
    var book = TriageBook()
    book.apply(.settle(date(2026, 11, 27, 10)), to: "done-thing")
    book.save(server: "h_one", to: defaults)
    #expect(TriageBook.load(server: "h_one", from: defaults) == book)
    #expect(TriageBook.load(server: "h_two", from: defaults).entries.isEmpty)
}

@Test func snoozeChoicesBeforeFive() {
    // A Friday afternoon.
    let choices = Triage.snoozeChoices(now: date(2026, 11, 27, 14, 30), calendar: calendar)
    #expect(choices.map(\.label) == ["For an Hour", "Until This Evening", "Until Tomorrow Morning", "Until Next Week"])
    #expect(choices[0].until == date(2026, 11, 27, 15, 30))
    #expect(choices[1].until == date(2026, 11, 27, 18))
    #expect(choices[2].until == date(2026, 11, 28, 9))
    #expect(choices[3].until == date(2026, 11, 30, 9))
}

@Test func snoozeChoicesInTheEvening() {
    let choices = Triage.snoozeChoices(now: date(2026, 11, 27, 17, 5), calendar: calendar)
    #expect(!choices.map(\.label).contains("Until This Evening"))
}

@Test func nextWeekFromAMondayIsAWeekAway() {
    let choices = Triage.snoozeChoices(now: date(2026, 11, 30, 8), calendar: calendar)
    #expect(choices.last?.until == date(2026, 12, 7, 9))
    // And from a Sunday, the next day.
    let sunday = Triage.snoozeChoices(now: date(2026, 11, 29, 8), calendar: calendar)
    #expect(sunday.last?.until == date(2026, 11, 30, 9))
}

@Test func monogramsMatchTheDesktops() {
    #expect(Monogram.letters(of: "hura") == "HA")
    #expect(Monogram.letters(of: "storefront") == "ST")
    #expect(Monogram.letters(of: "my-app") == "MA")
    #expect(Monogram.letters(of: "web2") == "W2")
    #expect(Monogram.letters(of: "東京") == "東")
    #expect(Monogram.letters(of: "--") == "PR")
    // T3 Code's hash: ((0*31+104)*31+117)... over "hura", mod 18.
    var index = 0
    for c in "hura".unicodeScalars { index = (index * 31 + Int(c.value)) % 18 }
    #expect(Monogram.colorIndex(of: "  Hura ") == index)
}
