import Foundation
import Testing
@testable import HuraKit

/// Images: a file read whole, one a tool answered with, and the paths a reply
/// may name.
struct PictureTests {
    @Test func aFileCarriesItsImageAndAnOlderServerNone() throws {
        let png = #"{"path":"a.png","text":"","bytes":4,"truncated":false,"binary":true,"image":{"mime":"image/png","data":"iVBORw=="}}"#
        let f = try JSONDecoder().decode(FileText.self, from: Data(png.utf8))
        #expect(f.image == Picture(mime: "image/png", data: "iVBORw=="))
        #expect(f.image?.bytes == Data([0x89, 0x50, 0x4E, 0x47]))

        let old = #"{"path":"a.png","text":"","bytes":4,"truncated":false,"binary":true}"#
        #expect(try JSONDecoder().decode(FileText.self, from: Data(old.utf8)).image == nil)
    }

    @Test func aToolResultKeepsItsImages() throws {
        let raw = #"{"seq":3,"at":0,"ev":{"kind":"sdk","msg":{"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw=="}},{"type":"text","text":"a screenshot"}]}]}}}}"#
        let entry = try #require(ChatTranscript.entry(JSONDecoder().decode(JSONValue.self, from: Data(raw.utf8))))
        let block = try #require(entry.blocks?.first)
        #expect(block.pictures == [Picture(mime: "image/png", data: "iVBORw==")])
        #expect(block.text == "a screenshot")
    }

    @Test(arguments: [
        ("docs/shot.png", "docs/shot.png"),
        ("./shot.png", "shot.png"),
        ("/sandbox/repo/out/a%20b.png", "out/a b.png"),
    ])
    func aWorkingCopyPathIsRead(src: String, path: String) {
        #expect(Picture.workingPath(src) == path)
    }

    @Test(arguments: ["https://example.com/x.png", "data:image/png;base64,AA", "file:///etc/x", "/etc/passwd", ""])
    func anythingElseIsNot(src: String) {
        #expect(Picture.workingPath(src) == nil)
    }
}
