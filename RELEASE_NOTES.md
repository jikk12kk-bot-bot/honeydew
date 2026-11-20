Honeydew 0.0.7: the sound modes in SETTINGS on Android.

- Settings (Android). `Sound mode:` now shows under Audio, with FL Studio Mobile's modes: `FAST`, `SAFE`, `ULTRASAFE` and `NATIVE` (the default), with `Enable low latency mode` under it for `NATIVE`. 0.0.6 added the modes but left the row hidden.
- Audio buffer size (Android). The slider now works on top of the sound mode instead of replacing it: the mode keeps its own stream, and its buffer is never smaller than the slider's size. At its left end (`Mode's own`) the sound mode's buffer applies alone.
- Background notification (Android 13 and up). The app asks once for permission to show notifications, so the "Running in the background" notification appears while it plays or exports in the background.
- Settings (desktop, web, iOS). `Enable low latency mode` no longer shows there: it is Android's (Oboe); those keep their own audio.
