import Testing
@testable import Hura

@Test func ctrlTypesALettersControlCharacter() {
    #expect(TerminalInput.modified("c", by: .ctrl) == .write("\u{03}"))
    #expect(TerminalInput.modified("C", by: .ctrl) == .write("\u{03}"))
    #expect(TerminalInput.modified("a", by: .ctrl) == .write("\u{01}"))
    #expect(TerminalInput.modified("z", by: .ctrl) == .write("\u{1A}"))
    // Only the first character counts, as in T3 Code.
    #expect(TerminalInput.modified("ls", by: .ctrl) == .write("\u{0C}"))
}

@Test func ctrlMapsThePunctuationAroundTheLetters() {
    #expect(TerminalInput.ctrl("@") == "\u{00}")
    #expect(TerminalInput.ctrl("[") == "\u{1B}")
    #expect(TerminalInput.ctrl("\\") == "\u{1C}")
    #expect(TerminalInput.ctrl("]") == "\u{1D}")
    #expect(TerminalInput.ctrl("^") == "\u{1E}")
    #expect(TerminalInput.ctrl("_") == "\u{1F}")
    #expect(TerminalInput.ctrl("?") == "\u{7F}")
    #expect(TerminalInput.ctrl("1") == "1")
    #expect(TerminalInput.ctrl("\t") == "\t")
    #expect(TerminalInput.ctrl("") == "")
    #expect(TerminalInput.ctrl("é") == "é")
}

@Test func altSendsEscapeBeforeTheKey() {
    #expect(TerminalInput.modified("b", by: .alt) == .write("\u{1B}b"))
    #expect(TerminalInput.modified("\u{7F}", by: .alt) == .write("\u{1B}\u{7F}"))
    #expect(TerminalInput.modified("\u{1B}", by: .alt) == .write("\u{1B}\u{1B}"))
}

@Test func aModifiedArrowSaysWhichModifier() {
    #expect(TerminalInput.modified("\u{1B}[A", by: .ctrl) == .write("\u{1B}[1;5A"))
    #expect(TerminalInput.modified("\u{1B}[D", by: .ctrl) == .write("\u{1B}[1;5D"))
    #expect(TerminalInput.modified("\u{1B}[C", by: .alt) == .write("\u{1B}[1;3C"))
    #expect(TerminalInput.modified("\u{1B}[B", by: .alt) == .write("\u{1B}[1;3B"))
}

@Test func ctrlVPastesThePhonesClipboard() {
    #expect(TerminalInput.modified("v", by: .ctrl) == .paste)
    #expect(TerminalInput.modified("V", by: .ctrl) == .paste)
    #expect(TerminalInput.modified("v", by: .alt) == .write("\u{1B}v"))
}

@Test func pasteTakesOutControlBytesAndEndsLinesWithReturn() {
    #expect(TerminalInput.paste("echo hi\nls\r\npwd\r") == "echo hi\rls\rpwd\r")
    #expect(TerminalInput.paste("a\u{1B}[31mb\u{03}c\u{7F}") == "a [31mb c ")
    #expect(TerminalInput.paste("tab\there") == "tab\there")
    #expect(TerminalInput.paste("\u{0B}\u{0C}\u{00}") == "   ")
    #expect(TerminalInput.paste("naïve ✓") == "naïve ✓")
}

@Test func pasteIsBracketedWhenTheProgramAsks() {
    #expect(TerminalInput.paste("one\ntwo", bracketed: true) == "\u{1B}[200~one\rtwo\u{1B}[201~")
    // An escape in the text can't close the bracket early.
    #expect(TerminalInput.paste("x\u{1B}[201~y", bracketed: true) == "\u{1B}[200~x [201~y\u{1B}[201~")
}

@Test func theBarHasT3CodesKeysInItsOrder() {
    #expect(TerminalKey.bar.map(\.label) == ["↑", "↓", "←", "→", "esc", "ctrl", "alt", "tab", "clear", "~", "|", "/", "-"])
    #expect(TerminalInput.clear == "\u{0C}")
}
