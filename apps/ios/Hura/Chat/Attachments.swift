import HuraKit
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// Files to send with a message, from the photo library or Files. The chat
/// host saves each one in the sandbox and shows a picture to the model as
/// well (`hura-agent.mjs`); what this side does is make a picture one the
/// model takes. A photo is HEIC and twelve megapixels, and the model takes
/// PNG, JPEG, GIF and WebP, and gains nothing past about two thousand pixels.
enum Attachments {
    /// Pictures sent as they are.
    static let taken: [UTType] = [.png, .jpeg, .gif, .webP]
    static let longSide: CGFloat = 2048

    /// A photo from the library, as a JPEG no longer than `longSide`.
    static func photo(_ data: Data, name: String) -> ChatUpload? {
        guard let image = UIImage(data: data) else { return nil }
        let scale = min(1, longSide / max(image.size.width, image.size.height))
        let size = CGSize(width: (image.size.width * scale).rounded(), height: (image.size.height * scale).rounded())
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        let scaled = UIGraphicsImageRenderer(size: size, format: format).image { _ in
            image.draw(in: CGRect(origin: .zero, size: size))
        }
        guard let jpeg = scaled.jpegData(compressionQuality: 0.85) else { return nil }
        return ChatUpload(name: name, mime: "image/jpeg", bytes: jpeg)
    }

    /// A file picked in Files, whole; a picture the model doesn't take as
    /// it is, such as a HEIC, is made a JPEG.
    static func file(_ url: URL) throws -> ChatUpload {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let data = try Data(contentsOf: url)
        let type = UTType(filenameExtension: url.pathExtension)
        if let type, type.conforms(to: .image), !taken.contains(where: type.conforms(to:)),
           let jpeg = photo(data, name: url.deletingPathExtension().lastPathComponent + ".jpg") {
            return jpeg
        }
        return ChatUpload(name: url.lastPathComponent, mime: type?.preferredMIMEType ?? "application/octet-stream",
                          bytes: data)
    }
}

/// The files waiting in the composer: a picture as itself, anything else by
/// name, each with a way out of the message.
struct AttachedStrip: View {
    let files: [ChatUpload]
    let remove: (ChatUpload) -> Void

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(files) { f in
                    ZStack(alignment: .topTrailing) {
                        chip(f)
                        Button { remove(f) } label: {
                            Image(systemName: "xmark.circle.fill")
                                .symbolRenderingMode(.palette)
                                .foregroundStyle(.white, .gray)
                        }
                        .offset(x: 6, y: -6)
                        .accessibilityLabel("Remove \(f.name)")
                    }
                }
            }
            .padding(.horizontal, 14)
            .padding(.top, 10)
        }
    }

    @ViewBuilder
    private func chip(_ f: ChatUpload) -> some View {
        if f.mime.hasPrefix("image/"), let data = Data(base64Encoded: f.data), let image = UIImage(data: data) {
            Image(uiImage: image)
                .resizable()
                .scaledToFill()
                .frame(width: 56, height: 56)
                .clipShape(RoundedRectangle(cornerRadius: 8))
        } else {
            Label(f.name, systemImage: "doc")
                .font(.caption)
                .lineLimit(1)
                .frame(maxWidth: 160, minHeight: 56)
                .padding(.horizontal, 8)
                .background(Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
        }
    }
}
