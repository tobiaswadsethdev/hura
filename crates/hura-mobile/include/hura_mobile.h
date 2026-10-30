// hura-core's tracker client, for the iOS app. See crates/hura-mobile.
#pragma once

// One request as JSON, `{"op": ..., ...}`, and its answer as JSON:
// `{"ok": ...}` or `{"error": "..."}`. Blocks until the tracker answers, so
// call it off the main thread. The answer is freed with hura_free.
char *hura_tracker(const char *request);

void hura_free(char *answer);
