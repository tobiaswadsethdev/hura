# gethura.app

The landing page: plain HTML, CSS and one script, no build step. Preview it with

```sh
python3 -m http.server -d site 8000
```

and serve the directory as it is from anywhere that serves static files.

- **The download button** is `download.js`. It guesses the visitor's platform
  from the user agent, then asks GitHub's API for the newest release and points
  the button and the download grid at its files, matched by the names
  `.github/workflows/release.yml` gives them. A renamed asset needs its pattern
  in `ASSETS` changed too. Without the script, or without an answer from
  GitHub, every link goes to the releases page.
- **The iPhone app** has no public link yet. When it is on the App Store, set
  `APP_STORE` in `download.js` and both iPhone buttons start pointing there.
- **The screenshots** are of the real apps, run against `scripts/fake-hurad.py`,
  which stands in for a server and for hura cloud and prints how to launch each
  app against it. `desktop-*.png` are the desktop window, captured with
  `screencapture -o -l <window id>` at 1280×800 points; `ios-*.png` are an
  iPhone 17 Pro simulator's, scaled to 645 pixels wide. The desktop selects the
  first worktree in the list, which `FAKE_FIRST=<name>` decides.
- **The icon** is `apps/desktop/src-tauri/icons/icon.svg` with its viewBox
  cropped to the tile; the PNGs are scaled from the iOS app's `icon-1024.png`.
