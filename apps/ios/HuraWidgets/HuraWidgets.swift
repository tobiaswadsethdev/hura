import SwiftUI
import WidgetKit

@main
struct HuraWidgets: WidgetBundle {
    var body: some Widget {
        AgentsActivityWidget()
        // A session's own activity, as an app from before the one for every
        // session showed it, drawn until the app ends it on launch.
        SessionActivityWidget()
    }
}
