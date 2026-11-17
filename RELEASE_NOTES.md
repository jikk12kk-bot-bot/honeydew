Honeydew 0.5.5 is the fork's first release from this repository: upstream Honeydew 0.5.4 with the fork's own features on top, and in-app updates from this repository's releases.

- Updates. UPDATES in the main menu checks this repository's releases (jikk12kk-bot-bot/honeydew) and installs this build's download; the sound library still comes from upstream's `sounds-*` releases.
- Main menu. A Quit button.
- Zoom. Wider horizontal and vertical zoom ranges in the playlist, piano roll, drum editor, step sequencer, audio and automation editors.
- Pickers. Folder shortcuts in every picker and in SONGS, kept per picker type; My Files is the whole user folder; the keyboard panel (transport bar, settings strip and keys) can stay in front of open pickers and keep working (Settings `Keyboard in front of pickers`).
- Tracks. Track folders (folder rows, rename, colour, mute and solo, drag into and out of folders, new tracks join their folder). Track Collapse, Hide track and Show hidden tracks are gone.
- Panels. The Displayer, Generate and Humanize panels can be moved; Generate leaves the transport's Play working under it, so its preview can be heard.
- Displayer. An image and video player under the views, with window and background opacity; MP4 render; an in-app MCP server (HTTP, over Wi-Fi too, on mobile).
- Sessions. Unsaved changes are kept across restarts in a working copy and restored at start; the recent-files write now finishes before the app exits.
- Lemon and Citrus. Lemon plugin presets (factory and Honeydew presets, in the native preset picker) and 50 Citrus factory presets.
- Piano roll. Duplicate.
- Upstream. Ports of upstream Honeydew 0.5.1 to 0.5.4.
- Fixes. The project builds again on every platform (it had stopped compiling after the track folders change).

Known: the automated tests that no longer matched the app (178 of them, mostly touch and layout tests of the playlist, drum editor and step sequencer) were removed and are to be rewritten.
