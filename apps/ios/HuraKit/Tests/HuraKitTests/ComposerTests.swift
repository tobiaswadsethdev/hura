import Foundation
import Testing
@testable import HuraKit

private func json(_ s: String) -> JSONValue {
    try! JSONDecoder().decode(JSONValue.self, from: Data(s.utf8))
}

private func entries(_ raw: [String]) -> [TranscriptEntry] {
    raw.map(json).compactMap(ChatTranscript.entry)
}

private func defaults() -> UserDefaults {
    let name = "hura.tests.\(UUID().uuidString)"
    let d = UserDefaults(suiteName: name)!
    d.removePersistentDomain(forName: name)
    return d
}

struct MentionTests {
    @Test func theLastWordIsAMentionWhenItStartsWithAnAt() {
        #expect(ComposerInput.mention("look at @src/ma")?.query == "src/ma")
        #expect(ComposerInput.mention("@")?.query == "")
        #expect(ComposerInput.mention("look at @src/main.rs and") == nil)
        #expect(ComposerInput.mention("mail me@example.com") == nil)
        #expect(ComposerInput.mention("") == nil)
    }

    @Test func pickingAPathReplacesTheMention() {
        #expect(ComposerInput.mentioning("src/main.rs", in: "look at @ma") == "look at @src/main.rs ")
        #expect(ComposerInput.mentioning("docs/", in: "@d") == "@docs/ ")
        #expect(ComposerInput.mentioning("x", in: "no mention") == "no mention")
    }
}

struct ForkTests {
    @Test func anEditForksFromTheLastThingSaidBeforeIt() {
        let all = entries([
            #"{"seq":1,"at":0,"ev":{"kind":"prompt","text":"one","uuid":"u1"}}"#,
            #"{"seq":2,"at":0,"ev":{"kind":"sdk","msg":{"type":"assistant","uuid":"a1","parent_tool_use_id":null,"message":{"content":[{"type":"text","text":"ok"}]}}}}"#,
            #"{"seq":3,"at":0,"ev":{"kind":"sdk","msg":{"type":"assistant","uuid":"sub","parent_tool_use_id":"toolu_9","message":{"content":[{"type":"text","text":"a subagent"}]}}}}"#,
            #"{"seq":4,"at":0,"ev":{"kind":"prompt","text":"two","uuid":"u2"}}"#,
            #"{"seq":5,"at":0,"ev":{"kind":"sdk","msg":{"type":"assistant","uuid":"a2","parent_tool_use_id":null,"message":{"content":[{"type":"text","text":"ok again"}]}}}}"#,
        ])
        #expect(all[1].uuid == "a1")
        #expect(ChatTranscript.assistantBefore(all, seq: 1) == nil)
        #expect(ChatTranscript.assistantBefore(all, seq: 4) == "a1")
        #expect(ChatTranscript.assistantBefore(all, seq: 6) == "a2")
    }
}

struct BackgroundCommandTests {
    @Test func aBackgroundCallIsFollowedThroughItsTask() {
        let all = entries([
            #"{"seq":1,"at":1000,"ev":{"kind":"sdk","msg":{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"npm run dev","description":"Serve it","run_in_background":true}}]}}}}"#,
            #"{"seq":2,"at":1000,"ev":{"kind":"sdk","msg":{"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"Command running in background with ID: b1. Output is being written to: /tmp/b1.output","is_error":false}]}}}}"#,
            #"{"seq":3,"at":1000,"ev":{"kind":"sdk","msg":{"type":"system","subtype":"task_started","task_id":"b1","tool_use_id":"t1","task_type":"local_bash","description":"Serve it"}}}"#,
            #"{"seq":4,"at":1000,"ev":{"kind":"sdk","msg":{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"lychee docs","run_in_background":true}}]}}}}"#,
            #"{"seq":5,"at":1000,"ev":{"kind":"sdk","msg":{"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":"Command running in background with ID: b2.","is_error":false}]}}}}"#,
            #"{"seq":6,"at":1000,"ev":{"kind":"sdk","msg":{"type":"system","subtype":"task_started","task_id":"b2","tool_use_id":"t2","task_type":"local_bash","description":"Check links"}}}"#,
            #"{"seq":7,"at":5000,"ev":{"kind":"sdk","msg":{"type":"system","subtype":"task_notification","task_id":"b2","status":"completed","summary":"None broken.","output_file":"/tmp/b2.output"}}}"#,
            #"{"seq":8,"at":1000,"ev":{"kind":"sdk","msg":{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"t3","name":"Bash","input":{"command":"ls"}}]}}}}"#,
            #"{"seq":9,"at":1000,"ev":{"kind":"sdk","msg":{"type":"system","subtype":"task_started","task_id":"b3","task_type":"local_bash","description":"Orphan"}}}"#,
            #"{"seq":10,"at":1000,"ev":{"kind":"sdk","msg":{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"t4","name":"KillShell","input":{"shell_id":"b3"}}]}}}}"#,
            #"{"seq":11,"at":1000,"ev":{"kind":"sdk","msg":{"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","tool_use_id":"t4","content":"killed","is_error":false}]}}}}"#,
        ])
        let commands = BackgroundCommand.all(all)
        #expect(commands.map(\.id) == ["t1", "t2", "b3"])
        #expect(commands[0].taskID == "b1")
        #expect(commands[0].running)
        #expect(commands[0].output == "/tmp/b1.output")
        #expect(commands[0].title == "Serve it")
        #expect(commands[1].status == "completed")
        #expect(commands[1].summary == "None broken.")
        #expect(commands[1].output == "/tmp/b2.output")
        #expect(commands[1].description == "Check links")
        #expect(commands[1].ended == Date(timeIntervalSince1970: 5))
        #expect(commands[2].status == "stopped")
    }
}

struct DraftTests {
    @Test func aDraftIsKeptPerConversationAndForgottenEmpty() {
        let drafts = Drafts(defaults: defaults())
        let a = Drafts.key(server: "s", session: "w", conv: "agent")
        let b = Drafts.key(server: "s", session: "w", conv: "chat-2")
        drafts.write(a, "half a thought")
        #expect(drafts.read(a) == "half a thought")
        #expect(drafts.read(b) == "")
        drafts.write(a, "  ")
        #expect(drafts.read(a) == "")
    }

    @Test func anOldDraftIsDroppedWhenAnotherIsWritten() {
        let drafts = Drafts(defaults: defaults())
        drafts.write("old", "from before", now: Date(timeIntervalSince1970: 0))
        drafts.write("new", "today", now: Date(timeIntervalSince1970: 40 * 24 * 3600))
        #expect(drafts.read("old") == "")
        #expect(drafts.read("new") == "today")
    }
}

@MainActor
struct StashTests {
    @Test func theStashIsNewestFirstAndOutlivesItsObject() {
        let store = defaults()
        let stash = PromptStash(defaults: store)
        stash.put("first")
        stash.put("second")
        stash.put("   ")
        #expect(stash.all.map(\.text) == ["second", "first"])
        stash.take(stash.all[0].id)
        #expect(PromptStash(defaults: store).all.map(\.text) == ["first"])
    }

    @Test func theStashKeepsAHundred() {
        let stash = PromptStash(defaults: defaults())
        for i in 0..<105 { stash.put("\(i)") }
        #expect(stash.all.count == 100)
        #expect(stash.all.first?.text == "104")
    }
}

struct ScriptTests {
    @Test func scriptsAreKeptPerServerAndProject() {
        let scripts = ProjectScripts(defaults: defaults())
        scripts.write([ProjectScript(name: "Dev", command: "npm run dev"), ProjectScript(name: "Empty", command: " ")],
                      server: "s", project: "hura")
        #expect(scripts.read(server: "s", project: "hura").map(\.name) == ["Dev"])
        #expect(scripts.read(server: "t", project: "hura").isEmpty)
        scripts.write([], server: "s", project: "hura")
        #expect(scripts.read(server: "s", project: "hura").isEmpty)
    }
}
