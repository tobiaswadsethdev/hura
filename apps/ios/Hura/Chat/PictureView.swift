import HuraKit
import SwiftUI
import UIKit

/// Reads a working-copy path as an image, for the worktree a chat is about.
/// Unset where there is none, and then a reply's image is its alt text.
struct ReadPictureKey: EnvironmentKey {
    static let defaultValue: (@MainActor (String) async -> Picture?)? = nil
}

extension EnvironmentValues {
    var readPicture: (@MainActor (String) async -> Picture?)? {
        get { self[ReadPictureKey.self] }
        set { self[ReadPictureKey.self] = newValue }
    }
}

/// An image, fitted to the width it is given and no taller than `maxHeight`.
/// A tap opens it on its own, to zoom.
struct PictureView: View {
    let picture: Picture
    var maxHeight: CGFloat = 360
    @State private var image: UIImage?
    @State private var opened = false

    var body: some View {
        Group {
            if let image {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFit()
                    .frame(maxHeight: min(maxHeight, image.size.height))
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                    .onTapGesture { opened = true }
                    .accessibilityAddTraits(.isButton)
                    .fullScreenCover(isPresented: $opened) { PictureScreen(image: image) }
            } else {
                Label("Image", systemImage: "photo").font(.caption).foregroundStyle(.secondary)
            }
        }
        .task(id: picture) { image = await Self.decode(picture) }
    }

    /// Off the main actor: a screenshot is megabytes of base64.
    nonisolated static func decode(_ picture: Picture) async -> UIImage? {
        await Task.detached(priority: .userInitiated) { picture.bytes.flatMap(UIImage.init(data:)) }.value
    }
}

/// One image, full screen, pinched to zoom and dragged when zoomed.
struct PictureScreen: View {
    let image: UIImage
    @Environment(\.dismiss) private var dismiss
    @State private var scale: CGFloat = 1
    @State private var settled: CGFloat = 1
    @State private var offset: CGSize = .zero
    @State private var dragged: CGSize = .zero

    var body: some View {
        NavigationStack {
            Image(uiImage: image)
                .resizable()
                .scaledToFit()
                .scaleEffect(scale)
                .offset(offset)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color.black)
                .gesture(MagnifyGesture()
                    .onChanged { scale = max(1, settled * $0.magnification) }
                    .onEnded { _ in
                        settled = scale
                        if scale == 1 { withAnimation { offset = .zero; dragged = .zero } }
                    })
                .simultaneousGesture(DragGesture()
                    .onChanged { v in
                        guard scale > 1 else { return }
                        offset = CGSize(width: dragged.width + v.translation.width,
                                        height: dragged.height + v.translation.height)
                    }
                    .onEnded { _ in dragged = offset })
                .onTapGesture(count: 2) {
                    withAnimation {
                        scale = scale > 1 ? 1 : 2.5
                        settled = scale
                        if scale == 1 { offset = .zero; dragged = .zero }
                    }
                }
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } }
                    ToolbarItem(placement: .primaryAction) {
                        ShareLink(item: Image(uiImage: image), preview: SharePreview("Image", image: Image(uiImage: image)))
                    }
                }
                .toolbarBackground(.visible, for: .navigationBar)
                .toolbarColorScheme(.dark, for: .navigationBar)
        }
    }
}

/// An image a reply names: read from the working copy when it is a file
/// there, its alt text otherwise.
struct MarkdownPicture: View {
    let alt: String
    let src: String
    @Environment(\.readPicture) private var read
    @State private var picture: Picture?
    @State private var tried = false

    var body: some View {
        if let picture {
            PictureView(picture: picture)
        } else {
            Text(alt.isEmpty ? src : "[\(alt)]")
                .foregroundStyle(.secondary)
                .task(id: src) {
                    guard !tried, let read, let path = Picture.workingPath(src) else { return }
                    tried = true
                    picture = await read(path)
                }
        }
    }
}
