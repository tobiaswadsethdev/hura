import Foundation

/// Every conversation this phone has seen, kept on it, and searching them.
///
/// The phone's copy of what the desktop keeps (`hura_client::history` in
/// `crates/hura-client/src/history.rs`), in the same shape and by the same
/// rules, so the two answer a search alike. The transcript is the host's,
/// inside the sandbox, and it goes with the sandbox; a conversation's entries
/// are kept here as they arrive on its channel, and caught up from the host
/// before a session is deleted or a conversation closed, and when a turn ends
/// in one no screen had open.
///
/// One file of entries per conversation, appended to, under
/// `history/<server>/<session>/<conv>.jsonl` in Application Support, kept out
/// of the phone's backups as the pairings in the Keychain are: a transcript is
/// the code, and whatever the agent read. Pictures are taken out and a tool's
/// result cut to its start before an entry is written. A deleted session's
/// directory is renamed `<session>@<when>`, and an ended conversation's file
/// likewise, so a later one by the same name is another one here too.
///
/// Searched by reading: the text of each entry is kept in memory once read,
/// and each file read again from where it was when it grows. Every call runs
/// in order on one queue of its own, so the frames of a conversation are
/// written as they came, and the main thread never waits on a disk.
public final class ChatHistory: @unchecked Sendable {
    /// The phone's own.
    public static let shared = ChatHistory(directory: URL.applicationSupportDirectory.appending(path: "history"))

    /// How many results one search answers with, newest first.
    public static let limit = 200
    /// How much of a tool's result is kept: enough to read the conversation
    /// by, and none of what is searched.
    static let resultChars = 4_000

    /// A conversation, as the history files it.
    public struct Conversation: Hashable, Sendable {
        public var serverID: String
        public var session: String
        public var conv: String

        public init(serverID: String, session: String, conv: String) {
            self.serverID = serverID
            self.session = session
            self.conv = conv
        }
    }

    /// Who an entry is from.
    public enum Role: String, Sendable {
        case you, claude, agent, shell, error
    }

    /// One entry that matched.
    public struct Hit: Identifiable, Hashable, Sendable {
        public var serverID: String
        public var session: String
        /// Which of the sessions by that name, for `transcript`.
        public var sessionKey: String
        public var title: String?
        public var project: String?
        /// The session has been deleted, so the conversation is only here.
        public var removed: Bool
        public var conv: String
        public var convKey: String
        /// The conversation has ended, or its host started it again.
        public var ended: Bool
        public var seq: UInt64
        public var at: Date
        public var role: Role
        public var snippet: String
        public var tools: [String]
        public var files: [String]

        public var id: String { "\(serverID)\n\(sessionKey)\n\(convKey)\n\(seq)" }
    }

    public struct Found: Equatable, Sendable {
        public var hits: [Hit] = []
        /// There were more than `limit`.
        public var more = false

        public init(hits: [Hit] = [], more: Bool = false) {
            self.hits = hits
            self.more = more
        }
    }

    struct Meta: Codable, Equatable {
        var title: String?
        var project: String?
        /// Epoch milliseconds, once it was deleted.
        var removed: UInt64?
    }

    struct Doc {
        var seq: UInt64
        var at: UInt64
        var role: Role
        var text: String
        var tools: [String]
        var files: [String]
        var haystack: String
    }

    /// What has been read of one file.
    struct Read {
        var offset: UInt64 = 0
        var last: UInt64 = 0
        /// When each entry was, by seq, in order: what tells one transcript's
        /// entry 1 from another's.
        var stamps: [UInt64: UInt64] = [:]
        var docs: [Doc] = []
    }

    private let dir: URL
    private let queue = DispatchQueue(label: "hura.history")
    private var read: [URL: Read] = [:]
    private var metas: [URL: Meta] = [:]

    public init(directory: URL) {
        dir = directory
    }

    // MARK: Keeping

    /// A replay from a host: what it has after `since`, in frames of which
    /// `seq` is each one's last. A host whose transcript started again -- it
    /// has less than it was asked from, or an entry it sends again is not the
    /// one kept under that number -- has what was kept put away first.
    public func replay(_ c: Conversation, since: UInt64, seq: UInt64, entries: [JSONValue]) {
        queue.async { [self] in
            let path = file(c)
            let r = refresh(path)
            let another = entries.contains { e in
                guard let s = e["seq"]?.uint, s <= r.last, let at = e["at"]?.uint, let kept = r.stamps[s] else { return false }
                return kept != at
            }
            if seq < since || another { endLocked(c) }
            keepLocked(c, entries)
        }
    }

    /// Entries as they arrive. Any already kept are skipped, so an overlapping
    /// replay, or a second screen's copy of an entry, is harmless.
    public func keep(_ c: Conversation, _ entries: [JSONValue]) {
        queue.async { [self] in keepLocked(c, entries) }
    }

    /// The last entry kept of a conversation, to ask its host from.
    public func last(_ c: Conversation) async -> UInt64 {
        await run { $0.refresh($0.file(c)).last }
    }

    /// What a session is called now, for when it is gone. Written only when it
    /// changed, and only for a session with something kept.
    public func name(serverID: String, session: String, title: String?, project: String?) {
        queue.async { [self] in
            let d = sessionDir(serverID, session)
            let meta = Meta(title: title, project: project, removed: nil)
            guard metas[d] != meta, FileManager.default.fileExists(atPath: d.path) else { return }
            if write(meta, to: d) { metas[d] = meta }
        }
    }

    /// A conversation that has ended, put away under when it did.
    public func ended(_ c: Conversation) {
        queue.async { [self] in endLocked(c) }
    }

    /// A session that has been deleted: its conversations put away together.
    public func removed(serverID: String, session: String) {
        queue.async { [self] in
            let d = sessionDir(serverID, session)
            guard FileManager.default.fileExists(atPath: d.path) else { return }
            let when = Self.now()
            var meta = meta(d)
            meta.removed = when
            _ = write(meta, to: d)
            let to = d.deletingLastPathComponent().appending(path: "\(Self.safe(session))@\(when)")
            try? FileManager.default.moveItem(at: d, to: to)
            metas[d] = nil
            read = read.filter { !$0.key.path.hasPrefix(d.path + "/") }
        }
    }

    /// Waits for everything asked before it to be done.
    public func flush() async {
        await run { _ in () }
    }

    // MARK: Reading

    /// Everything kept of one conversation, for one that is nowhere else.
    public func transcript(serverID: String, sessionKey: String, convKey: String) async -> [JSONValue] {
        await run { h in
            let path = h.dir.appending(path: Self.safe(serverID)).appending(path: Self.safeKey(sessionKey))
                .appending(path: "\(Self.safeKey(convKey)).jsonl")
            guard let data = try? Data(contentsOf: path) else { return [] }
            return data.split(separator: UInt8(ascii: "\n")).compactMap {
                try? JSONDecoder().decode(JSONValue.self, from: Data($0))
            }
        }
    }

    /// Every entry that matches, newest first.
    public func search(_ text: String) async -> Found {
        await run { $0.searchLocked(text) }
    }

    /// Read everything kept, so the first search does not have to.
    public func warm() {
        queue.async { [self] in _ = searchLocked("\u{0}") }
    }

    // MARK: On the queue

    private func run<T: Sendable>(_ work: @escaping @Sendable (ChatHistory) -> T) async -> T {
        await withCheckedContinuation { c in
            queue.async { [self] in c.resume(returning: work(self)) }
        }
    }

    private func sessionDir(_ serverID: String, _ session: String) -> URL {
        dir.appending(path: Self.safe(serverID)).appending(path: Self.safe(session))
    }

    func file(_ c: Conversation) -> URL {
        sessionDir(c.serverID, c.session).appending(path: "\(Self.safe(c.conv)).jsonl")
    }

    /// Brings what is known of one file up to what is in it.
    func refresh(_ path: URL) -> Read {
        let size = (try? FileManager.default.attributesOfItem(atPath: path.path)[.size] as? UInt64) ?? 0
        var r = read[path] ?? Read()
        if size < r.offset { r = Read() }
        if size > r.offset, let handle = try? FileHandle(forReadingFrom: path) {
            defer { try? handle.close() }
            try? handle.seek(toOffset: r.offset)
            let data = (try? handle.read(upToCount: Int(size - r.offset))) ?? Data()
            // Only whole lines: one being written is read once it is finished.
            if let end = data.lastIndex(of: UInt8(ascii: "\n")) {
                let whole = data[data.startIndex...end]
                for line in whole.split(separator: UInt8(ascii: "\n")) {
                    guard let e = try? JSONDecoder().decode(JSONValue.self, from: Data(line)),
                          let seq = e["seq"]?.uint else { continue }
                    r.last = max(r.last, seq)
                    r.stamps[seq] = e["at"]?.uint ?? 0
                    if let d = Self.doc(e) { r.docs.append(d) }
                }
                r.offset += UInt64(whole.count)
            }
        }
        read[path] = r
        return r
    }

    private func keepLocked(_ c: Conversation, _ entries: [JSONValue]) {
        let path = file(c)
        var newest = refresh(path).last
        var lines = Data()
        for e in entries {
            guard let seq = e["seq"]?.uint, seq > newest,
                  let data = try? JSONEncoder().encode(Self.trimmed(e)) else { continue }
            newest = seq
            lines.append(data)
            lines.append(UInt8(ascii: "\n"))
        }
        guard !lines.isEmpty else { return }
        let fm = FileManager.default
        if !fm.fileExists(atPath: dir.path) {
            try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
            var root = dir
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try? root.setResourceValues(values)
        }
        try? fm.createDirectory(at: path.deletingLastPathComponent(), withIntermediateDirectories: true)
        if !fm.fileExists(atPath: path.path) {
            fm.createFile(atPath: path.path, contents: nil, attributes: [.posixPermissions: 0o600])
        }
        guard let handle = try? FileHandle(forWritingTo: path) else { return }
        defer { try? handle.close() }
        _ = try? handle.seekToEnd()
        try? handle.write(contentsOf: lines)
    }

    private func endLocked(_ c: Conversation) {
        let path = file(c)
        guard FileManager.default.fileExists(atPath: path.path) else { return }
        let to = path.deletingLastPathComponent().appending(path: "\(Self.safe(c.conv))@\(Self.now()).jsonl")
        try? FileManager.default.moveItem(at: path, to: to)
        read[path] = nil
    }

    private func meta(_ d: URL) -> Meta {
        if let m = metas[d] { return m }
        let m = (try? Data(contentsOf: d.appending(path: "meta.json")))
            .flatMap { try? JSONDecoder().decode(Meta.self, from: $0) } ?? Meta()
        metas[d] = m
        return m
    }

    private func write(_ meta: Meta, to d: URL) -> Bool {
        guard let data = try? JSONEncoder().encode(meta) else { return false }
        return (try? data.write(to: d.appending(path: "meta.json"), options: .atomic)) != nil
    }

    private func searchLocked(_ text: String) -> Found {
        let query = Query(text)
        guard !query.isEmpty else { return Found() }
        let fm = FileManager.default
        func dirs(_ u: URL) -> [URL] {
            ((try? fm.contentsOfDirectory(at: u, includingPropertiesForKeys: [.isDirectoryKey])) ?? [])
                .filter { (try? $0.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true }
        }
        var hits: [Hit] = []
        for serverDir in dirs(dir) {
            let serverID = Self.unsafe(serverDir.lastPathComponent)
            for sessionDir in dirs(serverDir) {
                let sessionKey = sessionDir.lastPathComponent
                let (session, _) = Self.keptName(sessionKey)
                let meta = meta(sessionDir)
                let files = ((try? fm.contentsOfDirectory(at: sessionDir, includingPropertiesForKeys: nil)) ?? [])
                    .filter { $0.pathExtension == "jsonl" }
                for path in files {
                    let convKey = path.deletingPathExtension().lastPathComponent
                    let (conv, ended) = Self.keptName(convKey)
                    for d in refresh(path).docs where query.matches(d) {
                        hits.append(Hit(serverID: serverID, session: session, sessionKey: sessionKey,
                                        title: meta.title, project: meta.project, removed: meta.removed != nil,
                                        conv: conv, convKey: convKey, ended: ended, seq: d.seq,
                                        at: Date(timeIntervalSince1970: Double(d.at) / 1000), role: d.role,
                                        snippet: Self.snippet(d.text, words: query.words), tools: d.tools, files: d.files))
                    }
                }
            }
        }
        hits.sort { $0.at != $1.at ? $0.at > $1.at : $0.seq > $1.seq }
        return Found(hits: Array(hits.prefix(Self.limit)), more: hits.count > Self.limit)
    }

    // MARK: Pure

    /// What of an entry is searched, or nil for one with nothing a person
    /// would look for. A tool's result is left out on purpose: it is the file
    /// read, and every word in the repository would match.
    static func doc(_ e: JSONValue) -> Doc? {
        guard let seq = e["seq"]?.uint, let ev = e["ev"] else { return nil }
        var role = Role.you
        var text = ""
        var tools: [String] = []
        var files: [String] = []
        switch ev["kind"]?.string {
        case "prompt": text = ev["text"]?.string ?? ""
        case "shell":
            role = .shell
            text = ev["command"]?.string ?? ""
            tools = ["shell"]
        case "error":
            role = .error
            text = ev["message"]?.string ?? ""
        case "sdk":
            guard let msg = ev["msg"], msg["type"]?.string == "assistant",
                  case let .array(content)? = msg["message"]?["content"] else { return nil }
            role = msg["parent_tool_use_id"].map { $0 != .null } == true ? .agent : .claude
            var said: [String] = []
            for b in content {
                switch b["type"]?.string {
                case "text": if let t = b["text"]?.string { said.append(t) }
                case "tool_use":
                    let name = b["name"]?.string ?? "tool"
                    if !tools.contains(name) { tools.append(name) }
                    let input = b["input"]
                    for k in ["file_path", "notebook_path", "path"] {
                        if let p = input?[k]?.string, !files.contains(p) { files.append(p) }
                    }
                    // Every one it has: a command is remembered by what it ran
                    // and by what it was for, "Run the search tests".
                    let what = ["command", "pattern", "url", "query", "description", "skill"]
                        .compactMap { input?[$0]?.string }
                    if !what.isEmpty { said.append("\(name): \(what.joined(separator: " · "))") }
                default: continue
                }
            }
            text = said.joined(separator: "\n")
        default:
            return nil
        }
        if text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && tools.isEmpty { return nil }
        let haystack = [text, tools.joined(separator: " "), files.joined(separator: " ")].joined(separator: "\n").lowercased()
        return Doc(seq: seq, at: e["at"]?.uint ?? 0, role: role, text: text, tools: tools, files: files, haystack: haystack)
    }

    /// An entry as it is kept: a picture's bytes out, a sent file's too, and
    /// a tool's result cut to its start.
    static func trimmed(_ e: JSONValue) -> JSONValue {
        func cut(_ s: String) -> String {
            s.count > resultChars ? String(s.prefix(resultChars)) + "\n…" : s
        }
        func strip(_ v: JSONValue, inResult: Bool) -> JSONValue {
            switch v {
            case var .object(map):
                let kind = map["type"]?.string
                if kind == "image", case var .object(source)? = map["source"] {
                    source["data"] = nil
                    map["source"] = .object(source)
                }
                let inResult = inResult || kind == "tool_result"
                for (k, child) in map {
                    if inResult, k == "content" || k == "text", case let .string(s) = child {
                        map[k] = .string(cut(s))
                    } else {
                        map[k] = strip(child, inResult: inResult)
                    }
                }
                return .object(map)
            case let .array(items):
                return .array(items.map { strip($0, inResult: inResult) })
            default:
                return v
            }
        }
        guard case var .object(entry) = e, case var .object(ev)? = entry["ev"] else { return e }
        switch ev["kind"]?.string {
        case "sdk":
            if case var .object(msg)? = ev["msg"] {
                // The same result again, as Claude Code structured it.
                msg["tool_use_result"] = nil
                ev["msg"] = strip(.object(msg), inResult: false)
            }
        case "prompt":
            if case let .array(files)? = ev["files"] {
                ev["files"] = .array(files.map { f in
                    guard case var .object(o) = f else { return f }
                    o["data"] = .null
                    return .object(o)
                })
            }
        default: break
        }
        entry["ev"] = .object(ev)
        return .object(entry)
    }

    /// Words that must all be there, and `file:` and `tool:` to say where; a
    /// phrase in double quotes is one word.
    struct Query {
        var words: [String] = []
        var files: [String] = []
        var tools: [String] = []

        init(_ text: String) {
            var word = ""
            var quoted = false
            var started = false
            func end() {
                defer { word = ""; started = false }
                guard started else { return }
                let w = word.lowercased()
                if w.hasPrefix("file:"), w.count > 5 { files.append(String(w.dropFirst(5))) }
                else if w.hasPrefix("tool:"), w.count > 5 { tools.append(String(w.dropFirst(5))) }
                else if !w.isEmpty { words.append(w) }
            }
            for ch in text {
                if ch == "\"" { quoted.toggle(); started = true }
                else if ch.isWhitespace && !quoted { end() }
                else { word.append(ch); started = true }
            }
            end()
        }

        var isEmpty: Bool { words.isEmpty && files.isEmpty && tools.isEmpty }

        func matches(_ d: Doc) -> Bool {
            func has(_ list: [String], _ want: String) -> Bool { list.contains { $0.lowercased().contains(want) } }
            return words.allSatisfy { d.haystack.contains($0) }
                && files.allSatisfy { has(d.files, $0) }
                && tools.allSatisfy { has(d.tools, $0) }
        }
    }

    /// The text around the first word looked for, on one line.
    static func snippet(_ text: String, words: [String]) -> String {
        let flat = Array(text.split(whereSeparator: \.isWhitespace).joined(separator: " "))
        let lower = Array(String(flat).lowercased())
        var at = 0
        if lower.count == flat.count {
            at = words.compactMap { w -> Int? in
                let w = Array(w)
                guard !w.isEmpty, w.count <= lower.count else { return nil }
                return (0...(lower.count - w.count)).first { Array(lower[$0..<($0 + w.count)]) == w }
            }.min() ?? 0
        }
        let start = max(0, at - 50)
        let end = min(flat.count, start + 180)
        var out = String(flat[start..<end])
        if start > 0 { out = "…" + out }
        if end < flat.count { out += "…" }
        return out
    }

    /// A name made safe to be a file's, as `history.rs` makes it.
    static func safe(_ name: String) -> String {
        var out = ""
        for b in name.utf8 {
            let ok = (b >= 0x30 && b <= 0x39) || (b >= 0x41 && b <= 0x5a) || (b >= 0x61 && b <= 0x7a)
                || ((b == UInt8(ascii: "-") || b == UInt8(ascii: "_") || b == UInt8(ascii: ".")) && !(out.isEmpty && b == UInt8(ascii: ".")))
            out += ok ? String(UnicodeScalar(b)) : String(format: "%%%02x", b)
        }
        return out
    }

    static func unsafe(_ file: String) -> String {
        let bytes = Array(file.utf8)
        var out: [UInt8] = []
        var i = 0
        while i < bytes.count {
            if bytes[i] == UInt8(ascii: "%"), i + 2 < bytes.count,
               let b = UInt8(String(decoding: bytes[(i + 1)...(i + 2)], as: UTF8.self), radix: 16) {
                out.append(b)
                i += 3
            } else {
                out.append(bytes[i])
                i += 1
            }
        }
        return String(decoding: out, as: UTF8.self)
    }

    /// A key this made, made safe again, so nothing in one steps outside.
    static func safeKey(_ key: String) -> String {
        if let at = key.firstIndex(of: "@") {
            let when = key[key.index(after: at)...]
            if !when.isEmpty, when.allSatisfy(\.isASCII), when.allSatisfy(\.isNumber) {
                return "\(safe(unsafe(String(key[..<at]))))@\(when)"
            }
        }
        return safe(unsafe(key))
    }

    /// A name with when it was put away taken off.
    static func keptName(_ key: String) -> (String, Bool) {
        if let at = key.firstIndex(of: "@") { return (unsafe(String(key[..<at])), true) }
        return (unsafe(key), false)
    }

    static func now() -> UInt64 { UInt64(Date().timeIntervalSince1970 * 1000) }
}

extension ChatHistory {
    /// Asks a session's hosts for what this phone has not kept, on a
    /// connection of its own: every conversation at once, each done when its
    /// state arrives -- sent after the last of its replay -- or when it says
    /// there is no host yet. Bounded, since a delete waits for it.
    @MainActor
    public func catchUp(record: ServerRecord, session: String, conversations: [String],
                        transport: TransportFactory = Chat.network, within: Duration = .seconds(20)) async {
        guard !conversations.isEmpty else { return }
        let socket = transport(record)
        var waiting: [Int: (Conversation, UInt64)] = [:]
        for (i, conv) in conversations.enumerated() {
            let c = Conversation(serverID: record.serverID, session: session, conv: conv)
            let since = await last(c)
            let open: [String: RequestJSON] = [
                "do": .string("open"), "id": .number(i + 1),
                "channel": .object(["kind": .string("chat"), "session": .string(session),
                                    "conv": .string(conv), "since": .number(Int(since))]),
            ]
            guard let data = try? JSONEncoder().encode(open), (try? await socket.send(.text(data))) != nil else { break }
            waiting[i + 1] = (c, since)
        }
        let timer = Task { @MainActor in
            try? await Task.sleep(for: within)
            if !Task.isCancelled { socket.close(code: 1000) }
        }
        defer {
            timer.cancel()
            socket.close(code: 1000)
        }
        while !waiting.isEmpty {
            // A closed connection, or the timer closing it, ends it.
            guard let message = try? await socket.receive() else { return }
            guard case let .text(data) = message,
                  let frame = try? JSONDecoder().decode(JSONValue.self, from: data),
                  case let .number(n)? = frame["id"], let (c, since) = waiting[Int(n)] else { continue }
            switch frame["is"]?.string {
            case "chat":
                guard let f = frame["frame"] else { continue }
                switch f["t"]?.string {
                case "replay":
                    if case let .array(list)? = f["entries"] {
                        replay(c, since: since, seq: f["seq"]?.uint ?? 0, entries: list)
                    }
                case "entry":
                    if let e = f["entry"] { keep(c, [e]) }
                case "state", "notice", "error":
                    waiting[Int(n)] = nil
                default: continue
                }
            case "closed":
                waiting[Int(n)] = nil
            default:
                continue
            }
        }
    }
}

extension JSONValue {
    /// A whole number the protocol sends, such as a seq or a time.
    var uint: UInt64? {
        if case let .number(n) = self, n >= 0 { return UInt64(n) }
        return nil
    }
}
