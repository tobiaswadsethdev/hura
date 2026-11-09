// Makes the app icon from the desktop application's: iOS wants 1024 × 1024,
// square and opaque, and rounds the corners itself, so the desktop icon's
// tile is drawn with square corners and cropped to fill the square.
//
//   swift Tools/make-icon.swift ../desktop/src-tauri/icons/icon.svg Hura/Assets.xcassets/AppIcon.appiconset/icon-1024.png

import AppKit
import CoreGraphics

let args = CommandLine.arguments
guard args.count == 3, var svg = try? String(contentsOfFile: args[1], encoding: .utf8) else {
    FileHandle.standardError.write(Data("usage: make-icon.swift <icon.svg> <out.png>\n".utf8))
    exit(1)
}

// The tile is `<rect id="tile" … rx="…">`; with its corners squared there is
// nothing behind them to match.
guard let tile = svg.range(of: #"<rect id="tile"[^>]*>"#, options: .regularExpression) else {
    FileHandle.standardError.write(Data("make-icon.swift: no <rect id=\"tile\"> in \(args[1])\n".utf8))
    exit(1)
}
svg.replaceSubrange(tile, with: svg[tile].replacingOccurrences(of: #" rx="[^"]*""#, with: "", options: .regularExpression))
let source = NSImage(data: Data(svg.utf8))!

// The tile spans 100…924 of the icon's 1024: macOS's grid, with a margin
// for the shadow. Drawn so the tile fills the square, a little past its
// edge, whose antialiasing would otherwise be a dark hairline round it.
let size = 1024
let scale = CGFloat(size) / 820
let space = CGColorSpace(name: CGColorSpace.sRGB)!
let ctx = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0, space: space,
                    bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
NSGraphicsContext.current = NSGraphicsContext(cgContext: ctx, flipped: false)
NSGraphicsContext.current!.imageInterpolation = .high
source.draw(in: NSRect(x: -102 * scale, y: -102 * scale, width: CGFloat(size) * scale, height: CGFloat(size) * scale))

let out = NSBitmapImageRep(cgImage: ctx.makeImage()!)
try! out.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: args[2]))
