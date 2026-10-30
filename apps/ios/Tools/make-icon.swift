// Makes the app icon from the desktop application's: iOS wants 1024 × 1024,
// square and opaque, and rounds the corners itself, so the desktop icon's
// own rounded tile is cropped close and laid on its background colour.
//
//   swift Tools/make-icon.swift ../desktop/src-tauri/icons/icon.png Hura/Assets.xcassets/AppIcon.appiconset/icon-1024.png

import AppKit
import CoreGraphics

let args = CommandLine.arguments
guard args.count == 3,
      let source = NSImage(contentsOfFile: args[1])?.cgImage(forProposedRect: nil, context: nil, hints: nil)
else {
    FileHandle.standardError.write(Data("usage: make-icon.swift <icon.png> <out.png>\n".utf8))
    exit(1)
}

let size = 1024
let space = CGColorSpace(name: CGColorSpace.sRGB)!
let ctx = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0, space: space,
                    bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!

// The tile's colour, read from its middle-left edge, inside the rounding.
let rep = NSBitmapImageRep(cgImage: source)
let w = source.width, h = source.height
let edge = rep.colorAt(x: w * 8 / 100, y: h / 2)!.usingColorSpace(.sRGB)!
ctx.setFillColor(edge.cgColor)
ctx.fill(CGRect(x: 0, y: 0, width: size, height: size))

// Crop inside the tile, past its edge (about 4.5% in); what is left of its
// rounded corners falls under the corners iOS rounds off.
let inset = CGFloat(w) * 0.10
let crop = source.cropping(to: CGRect(x: inset, y: inset, width: CGFloat(w) - 2 * inset, height: CGFloat(h) - 2 * inset))!
ctx.interpolationQuality = .high
ctx.draw(crop, in: CGRect(x: 0, y: 0, width: size, height: size))

let out = NSBitmapImageRep(cgImage: ctx.makeImage()!)
try! out.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: args[2]))
