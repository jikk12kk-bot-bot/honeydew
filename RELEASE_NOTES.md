Honeydew 0.0.8: choose Android's audio driver and buffer yourself.

- Settings (Android). The audio is now set by two controls of its own, and `Sound mode:` fills them in as a preset:
  - `Audio driver:` picks which Android audio system plays the sound: `AAudio` (Android's newest and lowest-latency, through Oboe; OpenSL ES on Android 8.0), `OpenSL ES` (Android's older native audio) or `AudioTrack` (Android's oldest: the highest latency, the widest compatibility).
  - `Audio buffer size` now goes up to 10240 frames: Auto, 64, 128, 256, 512, 1024, 2048, 4096, 6144, 8192, 10240. Auto is the driver's smallest buffer, which grows by itself when the sound crackles; a set size stays as set.
  - `Sound mode:` presets, as FL Studio Mobile's: `NATIVE` = AAudio, Auto; `FAST` = OpenSL ES, Auto; `SAFE` = OpenSL ES, 1024 frames; `ULTRASAFE` = AudioTrack, 4096 frames. Changing the driver or the buffer yourself shows `CUSTOM`.
  - `Enable low latency mode` (an exclusive stream on Android's fastest path) shows only while the driver is AAudio, the one it applies to.
  If a driver cannot open on a phone, the app falls back to the next one (AudioTrack to Oboe, Oboe to AAudio).
- Settings (desktop). `Audio buffer size` goes up to 10240 frames too.
